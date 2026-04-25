use clap::{Args, Parser, Subcommand};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;
use vidlib_core::{
    format_user_error, AuditRecord, AuditRecordKind, LibraryFolder, ProgressSnapshot,
    ScanCheckpoint, SearchQuery, VidLibError, VidLibResult,
};
use vidlib_db::Database;
use vidlib_duplicates::group_duplicates;
use vidlib_fileops::{apply_plan, audit_records_for_manifest, plan_by_extension, undo_from_manifest};
use vidlib_metadata::{ffprobe_available, FfprobeMetadataProvider};
use vidlib_ml::{LocalHeuristicEngine, MlInferenceEngine, OnnxInferenceEngine};
use vidlib_scanner::{scan_path, CancellationToken, ScanOptions};

#[derive(Debug, Parser)]
#[command(
    name = "vidlib",
    about = "Local-first video library organizer",
    version
)]
struct Cli {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    AddLibrary(AddLibraryArgs),
    ListLibraries,
    Scan(ScanArgs),
    Search(SearchArgs),
    Duplicates,
    Tag(TagArgs),
    PlanMove(PlanMoveArgs),
    ApplyPlan(ApplyPlanArgs),
    Undo(UndoArgs),
}

#[derive(Debug, Args)]
struct AddLibraryArgs {
    path: PathBuf,
    #[arg(long, default_value_t = true)]
    recursive: bool,
}

#[derive(Debug, Args)]
struct ScanArgs {
    path: PathBuf,
    #[arg(long)]
    exact_hash: bool,
    #[arg(long)]
    skip_extension: Vec<String>,
    #[arg(long)]
    onnx_model: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct SearchArgs {
    #[arg(long)]
    text: Option<String>,
    #[arg(long)]
    tag: Vec<String>,
    #[arg(long)]
    extension: Option<String>,
}

#[derive(Debug, Args)]
struct TagArgs {
    text: String,
}

#[derive(Debug, Args)]
struct PlanMoveArgs {
    destination_root: PathBuf,
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ApplyPlanArgs {
    plan: PathBuf,
    #[arg(long)]
    manifest: PathBuf,
    #[arg(long)]
    confirm: bool,
}

#[derive(Debug, Args)]
struct UndoArgs {
    manifest: PathBuf,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{}", format_user_error(&error));
        std::process::exit(1);
    }
}

fn run() -> VidLibResult<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let cli = Cli::parse();
    let mut db = Database::open_default()?;

    match cli.command {
        Commands::AddLibrary(args) => {
            let folder = LibraryFolder {
                id: Uuid::new_v4(),
                path: fs::canonicalize(&args.path).unwrap_or(args.path),
                recursive: args.recursive,
            };
            db.add_library_folder(&folder)?;
            print_output(cli.json, &folder)?;
        }
        Commands::ListLibraries => {
            let folders = db.list_library_folders()?;
            print_output(cli.json, &folders)?;
        }
        Commands::Scan(args) => {
            if !ffprobe_available() {
                eprintln!("warning: ffprobe not found on PATH; media metadata will fall back to filesystem-only values");
            }
            let canonical_path = fs::canonicalize(&args.path).unwrap_or(args.path.clone());
            let library = db
                .find_library_by_path(&canonical_path)?
                .unwrap_or(LibraryFolder {
                    id: Uuid::new_v4(),
                    path: canonical_path.clone(),
                    recursive: true,
                });
            db.add_library_folder(&library)?;
            let cancellation = CancellationToken::new();
            let provider = FfprobeMetadataProvider;
            let onnx_engine = args
                .onnx_model
                .as_ref()
                .map(|path| OnnxInferenceEngine::from_model_path(path))
                .transpose()?;
            db.insert_audit_records(&[AuditRecord {
                id: Uuid::new_v4(),
                plan_id: None,
                kind: AuditRecordKind::ScanStarted,
                source_path: Some(canonical_path.clone()),
                destination_path: None,
                details: "scan started".into(),
                created_at: chrono::Utc::now(),
            }])?;
            let scan_result = scan_path(
                &canonical_path,
                &ScanOptions {
                    recursive: true,
                    compute_exact_hash: args.exact_hash,
                    compute_chunk_hash: true,
                    ignore_hidden: false,
                    skip_extensions: args.skip_extension,
                },
                &provider,
                |snapshot: ProgressSnapshot| {
                    println!("{}", snapshot.message);
                },
                Some(&cancellation),
            )?;
            let mut persisted = Vec::with_capacity(scan_result.videos.len());
            for mut video in scan_result.videos {
                let ml_tags = if let Some(engine) = &onnx_engine {
                    engine.describe_video(&video.path)?
                } else {
                    LocalHeuristicEngine.describe_video(&video.path)?
                };
                video.tags = ml_tags.into_iter().map(|tag| tag.label).collect();
                persisted.push(video);
            }
            db.upsert_videos(&persisted)?;
            db.insert_scan_warnings(&scan_result.warnings)?;
            db.save_checkpoint(&ScanCheckpoint {
                library_id: library.id,
                last_path: Some(canonical_path),
                scanned_files: persisted.len() as u64,
                updated_at: chrono::Utc::now(),
            })?;
            db.insert_audit_records(&[AuditRecord {
                id: Uuid::new_v4(),
                plan_id: None,
                kind: AuditRecordKind::ScanCompleted,
                source_path: Some(library.path.clone()),
                destination_path: None,
                details: format!("scan completed with {} indexed files", persisted.len()),
                created_at: chrono::Utc::now(),
            }])?;
            print_output(cli.json, &scan_result.summary)?;
        }
        Commands::Search(args) => {
            let query = SearchQuery {
                text: args.text,
                tags: args.tag,
                extension: args.extension,
            };
            let results = db.search(&query)?;
            print_output(cli.json, &results)?;
        }
        Commands::Duplicates => {
            let entries = db.all_videos()?;
            let groups = group_duplicates(&entries);
            print_output(cli.json, &groups)?;
        }
        Commands::Tag(args) => {
            let results = db.search(&SearchQuery {
                text: Some(args.text),
                tags: Vec::new(),
                extension: None,
            })?;
            print_output(cli.json, &results)?;
        }
        Commands::PlanMove(args) => {
            let entries = db.all_videos()?;
            let plan = plan_by_extension(&entries, &args.destination_root);
            let output = args
                .output
                .unwrap_or_else(|| PathBuf::from(format!("plan-{}.json", plan.id)));
            fs::write(&output, serde_json::to_vec_pretty(&plan)?)
                .map_err(|err| VidLibError::Io(format!("writing {}: {err}", output.display())))?;
            if cli.json {
                print_output(true, &plan)?;
            } else {
                println!("Dry-run plan written to {}", output.display());
            }
        }
        Commands::ApplyPlan(args) => {
            let plan = serde_json::from_slice(&fs::read(&args.plan)?)?;
            let manifest = apply_plan(&plan, &args.manifest, args.confirm)?;
            db.insert_audit_records(&audit_records_for_manifest(&manifest))?;
            print_output(cli.json, &manifest)?;
        }
        Commands::Undo(args) => {
            let manifest = serde_json::from_slice(&fs::read(&args.manifest)?)?;
            undo_from_manifest(&manifest)?;
            db.insert_audit_records(&[AuditRecord {
                id: Uuid::new_v4(),
                plan_id: Some(manifest.plan_id),
                kind: AuditRecordKind::PlanUndone,
                source_path: None,
                destination_path: None,
                details: format!("undo complete for plan {}", manifest.plan_id),
                created_at: chrono::Utc::now(),
            }])?;
            if cli.json {
                print_output(true, &manifest)?;
            } else {
                println!("Undo complete");
            }
        }
    }

    Ok(())
}

fn print_output<T: serde::Serialize + std::fmt::Debug>(json: bool, value: &T) -> VidLibResult<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        println!("{value:#?}");
    }
    Ok(())
}
