import {test,expect} from '@playwright/test';
async function mount(page,patch={}) {
 await page.goto('/');
 await page.evaluate(async patch=>{const {createRecipeEditor}=await import('/recipe/editor.js');const {defaultRecipe}=await import('/recipe/fields.js');document.body.replaceChildren();window.editor=createRecipeEditor({recipe:{...defaultRecipe(),...patch}});document.body.append(editor.element);},patch);
}
const parameters=page=>page.evaluate(()=>editor.getRecipe().rules[0].parameters);
test('every_rule_family_is_editable_without_JSON_and_exact_counters_survive',async({page})=>{
 await mount(page);
 expect(await page.getByLabel('Naming rule',{exact:true}).locator('option').evaluateAll(n=>n.map(x=>x.value))).toEqual(expect.arrayContaining(['text','regex','number','pad_numbers','date','template']));
 await page.getByLabel('Naming rule',{exact:true}).selectOption('number');
 await page.getByLabel('Counter start',{exact:true}).fill('9007199254740993');await page.getByLabel('Counter step',{exact:true}).fill('-3');await page.getByLabel('Counter padding',{exact:true}).fill('8');await page.getByLabel('Repeat each',{exact:true}).fill('2');await page.getByLabel('Counter representation',{exact:true}).selectOption('hex_upper');await page.getByLabel('Reset by original folder',{exact:true}).check();
 expect((await parameters(page)).rule).toMatchObject({start:'9007199254740993',step:'-3',padding:'8',repeat_each:'2',radix:'hex_upper',reset_by_folder:true});
 await page.getByLabel('Naming rule',{exact:true}).selectOption('pad_numbers');await page.getByLabel('Existing number padding').fill('6');expect(await parameters(page)).toEqual({type:'pad_numbers',padding:'6',scope:'all'});
 await page.getByLabel('Naming rule',{exact:true}).selectOption('regex');await page.getByText('Regex options',{exact:true}).click();await page.getByLabel('Regex pattern').fill('(?<=S)(\\d+)');await page.getByLabel('Regex replacement').fill('${1}');await page.getByLabel('Regex case sensitive').uncheck();expect((await parameters(page)).rule).toMatchObject({pattern:'(?<=S)(\\d+)',replacement:'${1}',case_sensitive:false});
});
test('text_operations_preserve_all_fields_and_explicit_extension_mode',async({page})=>{
 await mount(page);await page.getByLabel('Naming rule',{exact:true}).selectOption('text');
 const types=await page.getByLabel('Text operation',{exact:true}).locator('option').evaluateAll(n=>n.map(x=>x.value));
 expect(types).toEqual(expect.arrayContaining(['prefix','suffix','insert','replace','remove_characters','remove_range','crop','move_copy','reverse','trim','collapse_whitespace','translate','transliterate','case','remove']));
 await page.getByLabel('Text operation',{exact:true}).selectOption('case');await page.getByLabel('Case mode').selectOption('title');await page.getByLabel('Case exceptions').fill('BBC\niPhone');await page.getByLabel('Roman numerals').selectOption('upper');
 expect((await parameters(page)).operation).toEqual({type:'case',mode:'title',exceptions:['BBC','iPhone'],roman:'upper'});
 await page.getByLabel('Segment 1',{exact:true}).selectOption('extension');expect(await page.evaluate(()=>editor.getRecipe().rules[0].segment)).toEqual({type:'extension'});
 await page.getByLabel('Text operation',{exact:true}).selectOption('move_copy');await page.getByLabel('Substring start').fill('2');await page.getByLabel('Substring end').fill('-1');await page.getByLabel('Copy substring').check();expect((await parameters(page)).operation).toMatchObject({start:2,end:-1,copy:true});
});
test('template_picker_exposes_missing_actions_hashes_and_optional_groups',async({page})=>{
 await mount(page);expect(await page.getByLabel('Naming rule',{exact:true}).locator('option').evaluateAll(n=>n.map(x=>x.value))).toContain('template');
 await page.getByLabel('Naming rule',{exact:true}).selectOption('template');await page.getByRole('button',{name:'Add template field',exact:true}).click();
 await page.getByLabel('Template field 1',{exact:true}).selectOption('media');await page.getByLabel('Media property',{exact:true}).selectOption('title');await page.getByLabel('Missing field action',{exact:true}).selectOption('fallback');await page.getByLabel('Fallback text',{exact:true}).fill('Untitled');
 expect((await parameters(page)).parts[0]).toEqual({type:'field',value:{key:{type:'media',property:'title'},format:{type:'text'},fallback:'Untitled'}});
 await page.getByRole('button',{name:'Add optional group',exact:true}).click();await page.locator('.template-part').filter({has:page.getByText('Template part 2',{exact:true})}).getByRole('button',{name:'Add template field',exact:true}).click();await page.getByLabel('Template field 2.1',{exact:true}).selectOption('hash');await page.getByLabel('Hash algorithm').selectOption('sha512');expect((await parameters(page)).parts[1].value[0].value.key).toEqual({type:'hash',algorithm:'sha512'});
});
test('metadata_lazy_open_keeps_imported_intents_and_clear_removes_effect',async({page})=>{
 await mount(page,{metadata_changes:[{type:'timestamp_increment',field:'modified',start_ns:'9007199254740993',step_ns:'-1'}]});
 await expect(page.getByText('Filesystem metadata',{exact:true})).toHaveCount(1);await page.getByText('Filesystem metadata',{exact:true}).click();
 expect(await page.evaluate(()=>editor.getRecipe().metadata_changes)).toEqual([{type:'timestamp_increment',field:'modified',start_ns:'9007199254740993',step_ns:'-1'}]);
 await expect(page.getByLabel('Modified timestamp action')).toHaveValue('increment');await page.getByLabel('Modified increment start (ns)').fill('9007199254740995');await page.getByLabel('Modified timestamp action').selectOption('keep');expect(await page.evaluate(()=>editor.getRecipe().metadata_changes)).toEqual([]);
});
test('selection_companions_destination_and_date_controls_preserve_typed_intents',async({page})=>{
 await mount(page);await expect(page.getByText('Selection filters',{exact:true})).toHaveCount(1);await page.getByText('Selection filters',{exact:true}).click();await page.getByLabel('Maximum depth').fill('2');await page.getByLabel('Minimum size (bytes)').fill('9007199254740993');
 expect(await page.evaluate(()=>editor.getRecipe().selection_filter)).toMatchObject({max_depth:2,size:{min:'9007199254740993',max:null}});
 await page.getByLabel('Companion policy').selectOption('match_exact');expect(await page.evaluate(()=>editor.getRecipe().companions)).toEqual({type:'match_exact',associations:[]});
 await page.getByLabel('Naming rule',{exact:true}).selectOption('date');await page.getByLabel('Date action').selectOption('reformat');await page.getByLabel('Input date format').fill('%d-%m-%Y');await page.getByLabel('Output date format').fill('%Y%m%d');expect((await parameters(page)).operation.rule).toMatchObject({input_format:'%d-%m-%Y',output_format:'%Y%m%d'});
});
test('regex_options_are_lazy_and_collision_controls_are_explicit',async({page})=>{
 await mount(page,{rules:[{id:'regex',enabled:true,segment:{type:'stem'},parameters:{type:'regex',rule:{pattern:'x',replacement:'y',scope:'first',case_sensitive:true}}}]});
 await expect(page.getByLabel('Regex pattern')).toHaveCount(0);await page.getByText('Regex options',{exact:true}).click();await expect(page.getByLabel('Regex pattern')).toHaveValue('x');
 await page.getByLabel('Collision handling').selectOption('suffix');await page.getByLabel('Collision separator').fill('_');await page.getByLabel('Maximum collision attempts').fill('20');
 expect(await page.evaluate(()=>editor.getRecipe().collision_policy)).toEqual({type:'suffix',separator:'_',max_attempts:20});
});
test('keyboard_rule_change_retains_focus_and_draft_in_narrow_editor',async({page})=>{
 await page.setViewportSize({width:390,height:844});await mount(page);await page.getByLabel('Naming rule',{exact:true}).focus();await page.keyboard.press('ArrowDown');await expect(page.getByLabel('Naming rule',{exact:true})).toBeFocused();await expect(page.getByLabel('New stem',{exact:true})).toHaveCount(1);
 await page.keyboard.press('Tab');await page.keyboard.press('Control+A');await page.keyboard.type('keyboard');expect((await parameters(page)).value).toBe('keyboard');
});
test('invalid_tagged_rule_draft_survives_other_structural_edits',async({page})=>{
 const errors=[];page.on('pageerror',e=>errors.push(e.message));await mount(page);await page.getByLabel('Naming rule',{exact:true}).selectOption('number');await page.getByLabel('Rule 1 parameters',{exact:true}).fill('{"type":"number"}');await page.getByRole('button',{name:'Add rule',exact:true}).click();
 await expect(page.getByLabel('Rule 1 parameters',{exact:true})).toHaveValue('{"type":"number"}');await expect(page.getByLabel('Naming rule 2',{exact:true})).toHaveCount(1);expect(errors).toEqual([]);
});
test('replacement_translation_crop_and_date_parameters_are_editable_data',async({page})=>{
 await mount(page);await page.getByLabel('Naming rule',{exact:true}).selectOption('text');await page.getByLabel('Text operation',{exact:true}).selectOption('replace');await page.getByRole('button',{name:'Add character pair',exact:true}).click();await page.getByLabel('From 1',{exact:true}).fill('_');await page.getByLabel('To 1',{exact:true}).fill(' ');await page.getByLabel('Replacement matches').selectOption('first');await page.getByLabel('Replacement case sensitive').uncheck();expect((await parameters(page)).operation).toEqual({type:'replace',replacements:[{from:'_',to:' '}],scope:'first',case_sensitive:false});
 await page.getByLabel('Text operation',{exact:true}).selectOption('translate');await page.getByRole('button',{name:'Add character pair',exact:true}).click();await page.getByLabel('From 1',{exact:true}).fill('ä');await page.getByLabel('To 1',{exact:true}).fill('a');expect((await parameters(page)).operation.translations).toEqual([{from:'ä',to:'a'}]);
 await page.getByLabel('Text operation',{exact:true}).selectOption('crop');await page.getByLabel('Crop delimiter',{exact:true}).fill('-release');await page.getByLabel('Crop occurrence').fill('2');await page.getByLabel('Keep side').selectOption('after');await page.getByLabel('Keep crop delimiter').check();expect((await parameters(page)).operation).toEqual({type:'crop',delimiter:'-release',occurrence:2,side:'after',include_delimiter:true});
 await page.getByLabel('Naming rule',{exact:true}).selectOption('date');await page.getByLabel('Date source',{exact:true}).selectOption('recorded');await page.getByLabel('Date position',{exact:true}).selectOption('before');await page.getByLabel('Date position grapheme').fill('-1');await page.getByLabel('Output timezone',{exact:true}).selectOption('iana');await page.getByLabel('Output timezone name').fill('Europe/Paris');await page.getByLabel('Assumed input timezone',{exact:true}).selectOption('fixed_offset');await page.getByLabel('Assumed input timezone seconds').fill('3600');await page.getByLabel('Ambiguous local time').selectOption('latest');
 expect((await parameters(page)).operation).toMatchObject({source:'recorded',at:{type:'before',position:'-1'},rule:{timezone:{type:'iana',name:'Europe/Paris'},assume_timezone:{type:'fixed_offset',seconds:'3600'},ambiguity:'latest'}});
});
