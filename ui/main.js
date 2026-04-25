const invoke = window.__TAURI__?.core?.invoke;

async function refreshDashboard() {
  if (!invoke) {
    document.getElementById("stats").textContent = "Run inside Tauri to view live data.";
    return;
  }
  const data = await invoke("dashboard");
  document.getElementById("stats").innerHTML = `
    <div class="stat-grid">
      <div><strong>Libraries</strong><span>${data.libraries}</span></div>
      <div><strong>Videos</strong><span>${data.videos}</span></div>
      <div><strong>Duplicate Groups</strong><span>${data.duplicate_groups}</span></div>
    </div>
  `;
}

async function addLibrary() {
  const path = document.getElementById("libraryPath").value;
  await invoke("add_library", { path, recursive: true });
  await refreshDashboard();
}

async function scanLibrary() {
  const path = document.getElementById("libraryPath").value;
  const response = await invoke("run_scan", { path });
  document.getElementById("stats").insertAdjacentHTML("beforeend", `<p>Indexed ${response.indexed_videos} videos.</p>`);
}

async function runSearch() {
  const text = document.getElementById("searchText").value || null;
  const results = await invoke("search", { text });
  document.getElementById("searchResults").textContent = results;
}

async function generatePlan() {
  const destinationRoot = document.getElementById("destinationRoot").value;
  const plan = await invoke("generate_plan", { destinationRoot });
  document.getElementById("planResults").textContent = plan;
}

document.getElementById("refresh").addEventListener("click", refreshDashboard);
document.getElementById("addLibrary").addEventListener("click", addLibrary);
document.getElementById("scanLibrary").addEventListener("click", scanLibrary);
document.getElementById("runSearch").addEventListener("click", runSearch);
document.getElementById("generatePlan").addEventListener("click", generatePlan);

refreshDashboard();
