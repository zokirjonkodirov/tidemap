use std::{collections::{BTreeMap, BTreeSet, HashMap}, path::PathBuf, sync::Arc, time::{Duration,Instant}};
use tidemap_core::{WorkspaceCore,default_data_dir,source::{extract_diagrams,patch_diagram}};
use tidemap_model::*;
use tidemap_diagram::{ColorBy,Family,NodeId,Span,StyleSheet};
use tidemap_renderer::Engine;
use crate::{ui::{self,dialogs,palette::Command,tree},canvas::DiagramCanvas,editor::SourceEditor,source_map,style_file::{self,StyleFile},theme};

pub(crate) struct Buffer {
    pub(crate) document:DocumentSnapshot, base_content:String, revision:u64, pub(crate) saving:Option<String>,
    pub(crate) external:Option<ExternalVersion>, last_used:Instant,
}
pub(crate) struct ExternalVersion { content:Option<String>,hash:Option<String>,deleted:bool }
impl Buffer {pub(crate) fn dirty(&self)->bool{self.document.content!=self.base_content}}
pub(crate) struct Preview { pub(crate) artifact:Arc<RenderArtifact>, signature:String }
pub(crate) struct PendingRender { at:Instant,request:RenderRequest,key:String,signature:String }
pub(crate) struct ReadIntent { generation:u64,activate:bool,block:Option<String>,record:bool,external:bool }
#[derive(Clone,Copy,PartialEq)]pub(crate) enum Dialog { Settings,Help,Diagnostics,Close,Export }
/// A finished layout with diagnostics, kept back while the user is still typing.
pub(crate) struct Held { key:String,artifact:Arc<RenderArtifact>,signature:String }
/// What the canvas card offers.
#[derive(Clone,Copy)]pub(crate) enum CardAction { OpenSource,Retry }
/// Choices in the Export dialog.
pub(crate) struct ExportChoice { pub(crate) format:ExportFormat,pub(crate) scale:u32,pub(crate) background:bool,pub(crate) job:Option<u64> }

/// Command-line startup request.
#[derive(Default)]
pub struct StartOptions { pub open:Option<PathBuf>, pub shot:Option<ShotOptions> }
/// Screenshot dev mode: paint one diagram, save the window as a PNG and exit.
pub struct ShotOptions { pub path:PathBuf, pub folder:Option<PathBuf>, pub dark:bool, pub select:Option<String>, pub source:Option<String>, pub dialog:Option<String>, pub info:bool, pub palette:Option<String>, pub search:Option<String>, pub outline:bool, pub empty:bool }
/// `select`: label text of the node to select; `source`: open the source panel (`full` for the whole file);
/// `dialog`: `export`, `settings`, `help`, `close` or `diagnostics`; `folder`: open this folder as the workspace first;
/// `info`, `palette` (query), `search` (text) and `outline` open those surfaces; `empty`: no file and no workspace.
pub(crate) struct ShotState { path:PathBuf, started:Instant, requested:bool, frames:u32, select:Option<String>, source:Option<String>, dialog:Option<String>, info:bool, palette:Option<String>, search:Option<String>, outline:bool }
/// Seconds after which a screenshot is taken even if the diagram never settled.
const SHOT_TIMEOUT:Duration=Duration::from_secs(20);
/// How long a toast stays on screen.
pub(crate) const TOAST_LIFETIME:Duration=Duration::from_secs(4);
/// Frames a screenshot of a surface without a diagram waits before it is taken.
const SHOT_STATIC_FRAMES:u32=8;
/// Delay between the last edit and a new layout job.
const EDIT_DEBOUNCE_MS:u64=30;
/// An artifact with diagnostics stays hidden this long after the last edit when a clean one is shown.
const HOLD_AFTER_EDIT:Duration=Duration::from_millis(1200);
/// Frames the diagram must have been painted before a screenshot is taken.
const SHOT_STABLE_FRAMES:u32=3;
const FONT_MEDIUM:&[u8]=include_bytes!("../assets/fonts/Inter-Medium.ttf");

/// Whether a finished layout replaces the picture now. A layout with diagnostics is held back
/// while a clean one is on screen and the user is still typing, so the diagram does not jump.
pub fn should_show(new_diag_count:usize,have_clean_previous:bool,since_last_edit:Duration)->bool{
    new_diag_count==0||!have_clean_previous||since_last_edit>=HOLD_AFTER_EDIT
}

pub struct TidemapApp {
    pub(crate) core:Option<WorkspaceCore>, pub(crate) renderer:Engine, pub(crate) style:StyleSheet, pub(crate) shot:Option<ShotState>, pub(crate) settings:Settings, pub(crate) dark:bool,
    pub(crate) workspace:Option<WorkspaceSnapshot>,pub(crate) recents:Vec<RecentWorkspace>,pub(crate) recoveries:Vec<RecoveryRecord>,pub(crate) buffers:BTreeMap<String,Buffer>,
    pub(crate) active_file:String,pub(crate) active_block:String,pub(crate) state:WorkspaceState,pub(crate) canvas:DiagramCanvas,pub(crate) nav_travel:Option<(crate::canvas::Travel,String)>,pub(crate) editor:SourceEditor,pub(crate) docview:crate::markdown::view::DocView,
    pub(crate) source_text:String,pub(crate) editor_key:String,pub(crate) sidebar_open:bool,pub(crate) editor_open:bool,pub(crate) full_file:bool,
    pub(crate) tree_query:String,pub(crate) collapsed:BTreeSet<String>,pub(crate) palette_open:bool,pub(crate) palette_query:String,pub(crate) palette_index:usize,pub(crate) palette_fresh:bool,pub(crate) search_fresh:bool,pub(crate) info_anchor:egui::Rect,pub(crate) canvas_area:egui::Rect,pub(crate) theme_anchor:egui::Rect,pub(crate) page_opens:std::collections::VecDeque<Option<(String,egui::Rect)>>,ctx:egui::Context,
    pub(crate) diagram_search:String,pub(crate) search_open:bool,pub(crate) outline_open:bool,pub(crate) info_open:bool,pub(crate) dialog:Option<Dialog>,
    pub(crate) recovery_file:Option<String>,pub(crate) conflict_file:Option<String>,pub(crate) toast:Option<(String,Instant)>,pub(crate) search_hits:Vec<NodeId>,pub(crate) search_key:(String,String),pub(crate) search_cursor:Option<usize>,
    pub(crate) previews:HashMap<String,Preview>,pub(crate) diagnostics:HashMap<String,RenderDiagnostic>,pub(crate) pending_render:Option<PendingRender>,
    pub(crate) active_job:Option<(u64,String,String)>,pub(crate) next_job:u64,pub(crate) reads:HashMap<String,ReadIntent>,pub(crate) generation:u64,
    pub(crate) pending_restore:Option<(String,Option<String>)>,pub(crate) last_journal:Option<Instant>,pub(crate) last_persist:Option<Instant>,
    pub(crate) allow_close:bool,
    pub(crate) font_ready:bool,pub(crate) diagnostics_text:String,pub(crate) workspace_revision:u64,pub(crate) tree_cache:Option<tree::TreeCache>,
    pub(crate) style_file:StyleFile,pub(crate) style_confirm:bool,pub(crate) held:Option<Held>,pub(crate) clean_seen:BTreeSet<String>,pub(crate) last_edit:Instant,pub(crate) cursor_seen:Option<usize>,pub(crate) prefetch_due:bool,pub(crate) card_frames:u32,pub(crate) export:ExportChoice,
    /// Screenshot dev hook: a file inside the opened folder to activate once the index is ready.
    shot_open:Option<PathBuf>,
    /// Screenshot dev hook: the folder is opened but no file is picked for the user.
    shot_idle:bool,
    /// What the palette lists: files and diagrams, or projects.
    pub(crate) palette_mode:ui::palette::PaletteMode,
    /// Path of the tree row the keyboard is on.
    pub(crate) tree_cursor:Option<String>,
    /// Folders folded inside the search results (separate from the plain tree's folding).
    pub(crate) search_collapsed:BTreeSet<String>,
    /// A diagram just created on disk, to open once the index lists it.
    pub(crate) pending_new:Option<(String,Instant)>,
    /// Asks the sidebar to focus its search field or its tree next frame.
    pub(crate) focus_request:Option<ui::sidebar::FocusTarget>,
    /// The sidebar search or tree had keyboard focus last frame.
    pub(crate) sidebar_focused:bool,
    /// Files that appeared on disk while the project was open, with when they were seen;
    /// their rows glow briefly.
    pub(crate) appeared:HashMap<String,Instant>,
    /// The first full index of the open project has finished.
    workspace_settled:bool,
}

impl TidemapApp {
    pub fn new(cc:&eframe::CreationContext<'_>,options:StartOptions)->Self {
        let ctx=cc.egui_ctx.clone();let wake:Arc<dyn Fn()+Send+Sync>=Arc::new(move||ctx.request_repaint());
        let style_file=StyleFile::load(&default_data_dir());let style=style_file.sheet.clone();let font:Arc<[u8]>=Arc::from(FONT_MEDIUM);let renderer=Engine::new(wake.clone(),font,style.clone());
        let mut error=None;
        let settings_existed=default_data_dir().join("state.json").exists();
        let mut core=match WorkspaceCore::new(default_data_dir(),wake){Ok(core)=>Some(core),Err(reason)=>{error=Some(format!("Could not open app data: {reason}"));None}};
        let bootstrap=core.as_ref().map(|core|core.bootstrap());
        let mut settings=bootstrap.as_ref().map(|state|state.settings.clone()).unwrap_or_default();settings.theme=dialogs::initial_theme(settings_existed,settings.theme);let shot_mode=options.shot.is_some();if let Some(shot)=&options.shot{settings.theme=if shot.dark{Theme::Dark}else{Theme::Light};settings.sidebar_width=0.0;settings.editor_width=0.0;}
        let recents=bootstrap.as_ref().map(|state|state.recent_workspaces.clone()).unwrap_or_default();
        let recoveries=bootstrap.as_ref().map(|state|state.recoveries.clone()).unwrap_or_default();
        let mut workspace=None;let mut state=WorkspaceState::default();
        if let (Some(core),Some(root))=(core.as_mut(),bootstrap.and_then(|state|state.last_workspace).filter(|_|!shot_mode)) {
            match core.open_workspace(root){Ok(snapshot)=>{workspace=Some(snapshot);state=core.workspace_state().unwrap_or_default();},Err(reason)=>error=Some(format!("Could not reopen your last folder: {reason}"))}
        }
        let pending_restore=state.active_path.clone().map(|path|(path,state.active_block_id.clone()));
        let dark=theme::apply(&cc.egui_ctx,settings.theme);
        let shot_folder=options.shot.as_ref().and_then(|shot|shot.folder.clone());
        let shot_empty=options.shot.as_ref().is_some_and(|shot|shot.empty);
        let shot=options.shot.map(|shot|ShotState{path:shot.path,started:Instant::now(),requested:false,frames:0,select:shot.select,source:shot.source,dialog:shot.dialog,info:shot.info,palette:shot.palette,search:shot.search,outline:shot.outline});
        let mut app=Self {core,renderer,style,shot,settings,dark,workspace,recents,recoveries,buffers:BTreeMap::new(),active_file:String::new(),active_block:String::new(),editor_open:state.editor_open,full_file:state.full_file,state,
            canvas:DiagramCanvas::default(),nav_travel:None,editor:SourceEditor::default(),docview:Default::default(),source_text:String::new(),editor_key:String::new(),sidebar_open:true,tree_query:String::new(),collapsed:BTreeSet::new(),palette_open:false,palette_query:String::new(),palette_index:0,palette_fresh:false,search_fresh:false,info_anchor:egui::Rect::NOTHING,canvas_area:egui::Rect::NOTHING,theme_anchor:egui::Rect::NOTHING,page_opens:Default::default(),ctx:cc.egui_ctx.clone(),
            diagram_search:String::new(),search_open:false,outline_open:false,info_open:false,dialog:None,recovery_file:None,conflict_file:None,toast:error.map(|text|(text,Instant::now())),search_hits:Vec::new(),search_key:(String::new(),String::new()),search_cursor:None,previews:HashMap::new(),diagnostics:HashMap::new(),pending_render:None,active_job:None,next_job:1,reads:HashMap::new(),generation:0,pending_restore,last_journal:None,last_persist:None,allow_close:false,font_ready:false,diagnostics_text:String::new(),workspace_revision:1,tree_cache:None,
            style_file,style_confirm:false,held:None,clean_seen:BTreeSet::new(),last_edit:Instant::now(),cursor_seen:None,prefetch_due:false,card_frames:0,export:ExportChoice{format:ExportFormat::Svg,scale:2,background:true,job:None},shot_open:None,shot_idle:false,
            palette_mode:ui::palette::PaletteMode::Files,tree_cursor:None,search_collapsed:BTreeSet::new(),pending_new:None,focus_request:None,sidebar_focused:false,appeared:HashMap::new(),workspace_settled:false};
        app.workspace_settled=app.workspace.as_ref().is_some_and(|workspace|!workspace.indexing);
        // A file given on the command line wins over whatever the last session would restore.
        if let Some(folder)=shot_folder{app.open_folder(Some(folder));app.pending_restore=None;app.shot_idle=options.open.is_none();app.shot_open=options.open;}
        else if let Some(path)=options.open{app.pending_restore=None;app.open_external(Some(path));}
        app.shot_idle|=shot_empty;
        if let Some(source)=app.shot.as_ref().and_then(|shot|shot.source.clone()){app.editor_open=true;if source=="full"{app.full_file=true;}}
        app
    }
    pub(crate) fn notify(&mut self,text:impl Into<String>){self.toast=Some((text.into(),Instant::now()));}
    pub(crate) fn diagram_key(&self)->String{format!("{}:{}",self.active_file,self.active_block)}
    pub(crate) fn block(&self)->Option<&DiagramBlock>{self.buffers.get(&self.active_file)?.document.blocks.iter().find(|block|block.id==self.active_block)}
    pub(crate) fn signature(&self,source:&str)->String{source.to_owned()}
    pub(crate) fn active_preview(&self)->Option<&Preview>{self.previews.get(&self.diagram_key())}
    pub(crate) fn record_state(&mut self){
        self.state.active_file_id=(!self.active_file.is_empty()).then(||self.active_file.clone());self.state.active_block_id=(!self.active_block.is_empty()).then(||self.active_block.clone());self.state.active_path=self.buffers.get(&self.active_file).map(|buffer|buffer.document.path.clone());
        self.state.editor_open=self.editor_open;self.state.full_file=self.full_file;self.last_persist=Some(Instant::now());
    }
    pub(crate) fn journal_all(&mut self)->Result<(),String>{
        if let Some(core)=self.core.as_mut(){for (id,buffer) in &self.buffers{if buffer.dirty(){core.journal(id,&buffer.document.content,&buffer.document.disk_hash).map_err(|reason|reason.to_string())?;}else if buffer.document.recovery.is_none(){core.discard_recovery(id).map_err(|reason|reason.to_string())?;}}
            core.update_settings(self.settings.clone()).map_err(|reason|reason.to_string())?;core.save_workspace_state(self.state.clone()).map_err(|reason|reason.to_string())?;core.flush().map_err(|reason|reason.to_string())?;}
        self.last_journal=None;self.last_persist=None;Ok(())
    }
    pub(crate) fn open_folder(&mut self,root:Option<PathBuf>){
        let root=root.or_else(||rfd::FileDialog::new().set_title("Open a diagram project").pick_folder());let Some(root)=root else{return;};
        self.record_state();if let Err(reason)=self.journal_all(){self.notify(format!("Recovery could not be saved: {reason}"));return;}
        let result=self.core.as_mut().map(|core|core.open_workspace(root));
        match result {Some(Ok(snapshot))=>{
            self.generation+=1;self.workspace_revision+=1;self.tree_cache=None;self.tree_cursor=None;self.appeared.clear();self.workspace_settled=!snapshot.indexing;self.renderer.cancel();self.active_job=None;self.pending_render=None;self.reads.clear();self.buffers.clear();self.previews.clear();self.diagnostics.clear();self.held=None;self.clean_seen.clear();self.workspace=Some(snapshot);self.active_file.clear();self.active_block.clear();self.editor_key.clear();self.collapsed.clear();self.search_collapsed.clear();self.pending_new=None;self.tree_query.clear();
            self.state=self.core.as_ref().and_then(|core|core.workspace_state()).unwrap_or_default();self.editor_open=self.state.editor_open;self.full_file=self.state.full_file;self.pending_restore=self.state.active_path.clone().map(|path|(path,self.state.active_block_id.clone()));
            if let Some(core)=self.core.as_ref(){let bootstrap=core.bootstrap();self.recents=bootstrap.recent_workspaces;self.recoveries=bootstrap.recoveries;}self.restore_pending();
        },Some(Err(reason))=>self.notify(reason.to_string()),None=>self.notify("Local file storage is unavailable")}
    }
    pub(crate) fn open_external(&mut self,path:Option<PathBuf>){
        let path=path.or_else(||rfd::FileDialog::new().set_title("Open Markdown or Mermaid").add_filter("Markdown & Mermaid",&["md","mmd"]).pick_file());let Some(path)=path else{return;};
        for intent in self.reads.values_mut(){intent.activate=false;}
        match self.core.as_mut().map(|core|core.open_external(path)){Some(Ok(id))=>{self.reads.insert(id,ReadIntent{generation:self.generation,activate:true,block:None,record:true,external:false});},Some(Err(reason))=>self.notify(reason.to_string()),None=>{}}
    }
    pub(crate) fn open_document(&mut self,id:String,block:Option<String>,record:bool){
        self.palette_open=false;
        for intent in self.reads.values_mut(){intent.activate=false;}
        if self.buffers.contains_key(&id){self.activate(id,block,record);return;}
        self.reads.insert(id.clone(),ReadIntent{generation:self.generation,activate:true,block,record,external:false});if let Some(core)=self.core.as_mut(){core.request_read(id);}
    }
    pub(crate) fn activate(&mut self,id:String,wanted:Option<String>,record:bool){
        let Some(buffer)=self.buffers.get_mut(&id)else{return;};buffer.last_used=Instant::now();
        self.active_file=id.clone();let wanted=wanted.or_else(||self.state.selected_blocks.get(&id).cloned());let wanted=wanted.and_then(|wanted|{if let Some(title)=wanted.strip_prefix("title\0"){buffer.document.blocks.iter().find(|block|block.title==title).map(|block|block.id.clone())}else if let Some(index)=wanted.strip_prefix("index\0"){index.parse::<usize>().ok().and_then(|index|buffer.document.blocks.get(index)).map(|block|block.id.clone())}else{Some(wanted)}});self.active_block=buffer.document.blocks.iter().find(|block|Some(&block.id)==wanted.as_ref()).or_else(||buffer.document.blocks.first()).map(|block|block.id.clone()).unwrap_or_default();
        let page=!self.state.diagram_view&&!buffer.document.name.to_lowercase().ends_with(".mmd");
        if buffer.document.blocks.is_empty()&&!page{self.editor_open=true;self.full_file=true;}
        self.state.selected_blocks.insert(id.clone(),self.active_block.clone());
        if record {self.state.history.truncate(self.state.history_index.saturating_add(1));if !self.state.history.last().is_some_and(|entry|entry.file_id==id&&entry.block_id==self.active_block){self.state.history.push(NavigationEntry{file_id:id,block_id:self.active_block.clone()});if self.state.history.len()>150{self.state.history.remove(0);}self.state.history_index=self.state.history.len().saturating_sub(1);}}
        let travel=crate::canvas::navigation_travel(self.nav_travel.take(),&self.active_file);self.canvas.set_travel(travel,&self.active_block);
        self.canvas.restore_camera(self.state.views.get(&self.diagram_key()).and_then(|view|view.camera));self.canvas.selected=None;self.canvas.matches.clear();self.search_hits.clear();self.search_cursor=None;self.diagram_search.clear();self.editor_key.clear();self.sync_editor();self.schedule_render(false);self.record_state();self.prune_buffers();
    }
    pub(crate) fn select_block(&mut self,id:String){self.activate(self.active_file.clone(),Some(id),true);}
    pub(crate) fn step_diagram(&mut self,delta:isize){let target=self.buffers.get(&self.active_file).and_then(|buffer|{let index=buffer.document.blocks.iter().position(|block|block.id==self.active_block)?;buffer.document.blocks.get(index.checked_add_signed(delta)?).map(|block|block.id.clone())});if let Some(id)=target{self.nav_travel=Some((crate::canvas::Travel::from_step(delta),self.active_file.clone()));self.select_block(id);}}
    pub(crate) fn history_step(&mut self,delta:isize){if let Some(index)=self.state.history_index.checked_add_signed(delta){if let Some(entry)=self.state.history.get(index).cloned(){self.state.history_index=index;self.nav_travel=Some((crate::canvas::Travel::from_step(delta),entry.file_id.clone()));self.open_document(entry.file_id,Some(entry.block_id),false);}}}
    pub(crate) fn restore_pending(&mut self){
        if self.shot_open.is_some()||self.shot_idle{return;}
        if !self.active_file.is_empty()||self.reads.values().any(|intent|intent.activate){return;}
        let found=self.workspace.as_ref().and_then(|workspace|self.pending_restore.as_ref().and_then(|(path,block)|workspace.files.iter().find(|file|&file.path==path).map(|file|(file.id.clone(),block.clone()))).or_else(||(!workspace.indexing).then(||workspace.files.first().map(|file|(file.id.clone(),None))).flatten()));
        if let Some((id,block))=found{self.pending_restore=None;self.open_document(id,block,false);}
    }
    pub(crate) fn sync_editor(&mut self){
        let key=format!("{}:{}",self.active_file,if self.full_file{"file"}else{&self.active_block});if key==self.editor_key{return;}
        self.editor_key=key;self.cursor_seen=None;self.editor.reset();self.source_text=if self.full_file{self.buffers.get(&self.active_file).map(|buffer|buffer.document.content.trim_start_matches('\u{feff}').replace("\r\n","\n")).unwrap_or_default()}else{self.block().map(|block|block.source.replace("\r\n","\n")).unwrap_or_default()};
    }
    pub(crate) fn apply_source(&mut self){
        let Some(buffer)=self.buffers.get_mut(&self.active_file)else{return;};if buffer.document.read_only{return;}
        let content=if self.full_file{let text=if buffer.document.newline=="\r\n"{self.source_text.replace('\n',"\r\n")}else{self.source_text.clone()};Ok(if buffer.document.content.starts_with('\u{feff}'){format!("\u{feff}{text}")}else{text})}else{match buffer.document.blocks.iter().find(|block|block.id==self.active_block){Some(block)=>patch_diagram(&buffer.document.content,block,&self.source_text).map_err(|reason|reason.to_string()),None=>return}};
        match content {Ok(content)=>{if content==buffer.document.content{return;}self.last_edit=Instant::now();buffer.document.content=content;buffer.revision+=1;buffer.last_used=Instant::now();let previous=std::mem::take(&mut buffer.document.blocks);buffer.document.blocks=extract_diagrams(&buffer.document.content,&buffer.document.path,&previous);if !buffer.document.blocks.iter().any(|block|block.id==self.active_block){self.active_block=buffer.document.blocks.first().map(|block|block.id.clone()).unwrap_or_default();}self.state.selected_blocks.insert(self.active_file.clone(),self.active_block.clone());self.last_journal=Some(Instant::now());self.schedule_render(false);},Err(reason)=>self.notify(reason)}
    }
    pub(crate) fn schedule_render(&mut self,retry:bool){
        let Some(block)=self.block()else{self.renderer.cancel();self.active_job=None;self.pending_render=None;return;};
        let key=self.diagram_key();let source=block.source.clone();let id=block.id.clone();let signature=self.signature(&source);
        self.renderer.cancel();self.active_job=None;self.held=None;
        if self.previews.get(&key).is_some_and(|preview|preview.signature==signature){self.diagnostics.remove(&key);self.pending_render=None;return;}
        // A cached layout is installed in the same frame.
        if let Some(artifact)=self.renderer.cached(&id,&source){self.offer_artifact(key,artifact,signature);self.pending_render=None;return;}
        let job_id=self.next_job;self.next_job+=1;
        let delay=if retry{0}else{EDIT_DEBOUNCE_MS};
        self.pending_render=Some(PendingRender{at:Instant::now()+Duration::from_millis(delay),key,signature,request:RenderRequest{job_id,diagram_id:id,revision:self.buffers.get(&self.active_file).map(|buffer|buffer.revision).unwrap_or(0),source}});
    }
    /// Applies the hold-last-good rule to a finished layout of the active diagram.
    pub(crate) fn offer_artifact(&mut self,key:String,artifact:Arc<RenderArtifact>,signature:String){
        self.prefetch_due=true;
        if should_show(artifact.rendered.diagnostics.len(),self.clean_seen.contains(&key),self.last_edit.elapsed()){self.held=None;self.install_artifact(key,artifact,signature);}
        else{self.held=Some(Held{key,artifact,signature});}
    }
    /// Shows a held layout once the user has stopped typing for long enough.
    pub(crate) fn release_held(&mut self,ctx:&egui::Context){
        if self.held.is_none(){return;}
        let waited=self.last_edit.elapsed();
        if waited>=HOLD_AFTER_EDIT{if let Some(held)=self.held.take(){self.install_artifact(held.key,held.artifact,held.signature);}}
        else{ctx.request_repaint_after(HOLD_AFTER_EDIT-waited+Duration::from_millis(5));}
    }
    /// Starts layouts of the diagrams next to the active one so switching is instant.
    pub(crate) fn prefetch_neighbours(&mut self){
        let Some(buffer)=self.buffers.get(&self.active_file)else{return;};
        let blocks=&buffer.document.blocks;
        let Some(index)=blocks.iter().position(|block|block.id==self.active_block)else{return;};
        let revision=buffer.revision;
        let wanted:Vec<(String,String)>=[index.checked_sub(1),index.checked_add(1)].into_iter().flatten().filter_map(|at|blocks.get(at)).map(|block|(block.id.clone(),block.source.clone())).collect();
        for (diagram_id,source) in wanted{
            if self.renderer.cached(&diagram_id,&source).is_some(){continue;}
            let job_id=self.next_job;self.next_job+=1;
            self.renderer.prefetch(RenderRequest{job_id,diagram_id,revision,source});
        }
    }
    pub(crate) fn install_artifact(&mut self,key:String,artifact:Arc<RenderArtifact>,signature:String){
        if artifact.rendered.diagnostics.is_empty(){self.clean_seen.insert(key.clone());}
        self.previews.insert(key.clone(),Preview{artifact,signature});self.diagnostics.remove(&key);
        let mut total=self.previews.values().map(|preview|preview.artifact.bytes).sum::<usize>();let remove=self.previews.keys().filter(|other|*other!=&key).cloned().collect::<Vec<_>>();
        for other in remove{if total<=80*1024*1024{break;}if let Some(preview)=self.previews.remove(&other){total=total.saturating_sub(preview.artifact.bytes);}}
    }
    pub(crate) fn save(&mut self,id:String,copy:bool){
        let Some(buffer)=self.buffers.get(&id)else{return;};if buffer.saving.is_some(){return;}
        if !copy&&(buffer.external.is_some()||buffer.document.read_only){self.conflict_file=Some(id);return;}
        let content=buffer.document.content.clone();let hash=buffer.document.disk_hash.clone();let name=buffer.document.name.clone();
        if copy {let Some(path)=rfd::FileDialog::new().set_title("Save a copy").set_file_name(&name).save_file()else{return;};if let Some(core)=self.core.as_mut(){core.save_copy(id.clone(),content.clone(),path);}}
        else if let Some(core)=self.core.as_mut(){if let Err(reason)=core.journal(&id,&content,&hash){self.notify(reason.to_string());return;}core.request_save(SaveRequest{file_id:id.clone(),content:content.clone(),expected_hash:hash});}
        if let Some(buffer)=self.buffers.get_mut(&id){buffer.saving=Some(content);}
    }
    pub(crate) fn reload_conflict(&mut self,id:String){self.reads.insert(id.clone(),ReadIntent{generation:self.generation,activate:true,block:Some(self.active_block.clone()),record:false,external:false});if let Some(core)=self.core.as_mut(){let _=core.discard_recovery(&id);core.request_read(id.clone());}self.buffers.remove(&id);self.conflict_file=None;}
    pub(crate) fn prune_buffers(&mut self){
        let cost=|buffer:&Buffer|2*(buffer.document.content.len()+buffer.base_content.len())+buffer.document.blocks.iter().map(|block|block.source.len()+block.lines.len()*48).sum::<usize>();
        let mut bytes=self.buffers.values().map(cost).sum::<usize>();let mut clean=self.buffers.values().filter(|buffer|!buffer.dirty()).count();
        let mut candidates=self.buffers.iter().filter(|(id,buffer)|*id!=&self.active_file&&!buffer.dirty()&&buffer.saving.is_none()&&buffer.external.is_none()&&buffer.document.recovery.is_none()).map(|(id,buffer)|(id.clone(),buffer.last_used)).collect::<Vec<_>>();candidates.sort_by_key(|(_,time)|*time);
        for (id,_) in candidates {if bytes<=64*1024*1024&&clean<=24{break;}if let Some(buffer)=self.buffers.remove(&id){bytes=bytes.saturating_sub(cost(&buffer));clean=clean.saturating_sub(1);}}
    }
    pub(crate) fn save_settings(&mut self){if let Some(core)=self.core.as_mut(){if let Err(reason)=core.update_settings(self.settings.clone()){self.notify(reason.to_string());}}}
    /// Applies the style sheet read from `style.json`. A change of layout metrics lays everything out again.
    pub(crate) fn apply_style(&mut self){
        let relayout=self.style.layout_key()!=self.style_file.sheet.layout_key();
        self.style=self.style_file.sheet.clone();self.renderer.set_style(self.style.clone());
        if relayout{for preview in self.previews.values_mut(){preview.signature.clear();}self.schedule_render(false);}
    }
    pub(crate) fn poll(&mut self,ctx:&egui::Context){
        if self.style_file.poll(Instant::now()){self.apply_style();}
        ctx.request_repaint_after(Duration::from_secs(1));
        let events=self.core.as_mut().map(|core|core.poll()).unwrap_or_default();
        for event in events {match event {
            CoreEvent::Workspace(snapshot)=>{if self.workspace.as_ref().is_some_and(|workspace|workspace.id==snapshot.id){
                // Files that show up after the first full index are new on disk: mark them so
                // their rows glow for a moment.
                if self.workspace_settled{if let Some(old)=&self.workspace{let known:BTreeSet<&str>=old.files.iter().map(|file|file.id.as_str()).collect();for file in &snapshot.files{if !known.contains(file.id.as_str()){self.appeared.insert(file.id.clone(),Instant::now());}}}}
                if !snapshot.indexing{self.workspace_settled=true;}
                self.appeared.retain(|_,seen|seen.elapsed()<Duration::from_secs(3));
                self.workspace=Some(snapshot);self.workspace_revision+=1;self.tree_cache=None;self.restore_pending();self.open_pending_new();}},
            CoreEvent::Document{file_id,result}=>{
                let Some(intent)=self.reads.remove(&file_id)else{continue;};if intent.generation!=self.generation{continue;}
                match result {Ok(document)=>{
                    if intent.external {if let Some(buffer)=self.buffers.get_mut(&file_id){if document.disk_hash!=buffer.document.disk_hash{if buffer.dirty()||buffer.saving.is_some(){buffer.external=Some(ExternalVersion{content:Some(document.content),hash:Some(document.disk_hash),deleted:false});self.notify("The file changed on disk. Review the conflict before saving.");}else{
                        // Live reload. Diagram ids come from content, so match the new blocks
                        // against the old ones; otherwise an edited diagram would get a new id
                        // and the canvas would lose it (and its camera) mid-edit.
                        let previous=buffer.document.blocks.clone();let was=previous.iter().position(|block|block.id==self.active_block);
                        let mut document=document;document.blocks=extract_diagrams(&document.content,&document.path,&previous);
                        let revision=buffer.revision+1;*buffer=Buffer{base_content:document.content.clone(),document,revision,saving:None,external:None,last_used:Instant::now()};
                        if self.active_file==file_id{
                            let blocks=&buffer.document.blocks;
                            if !blocks.iter().any(|block|block.id==self.active_block){self.active_block=was.and_then(|index|blocks.get(index.min(blocks.len().saturating_sub(1)))).or_else(||blocks.first()).map(|block|block.id.clone()).unwrap_or_default();self.state.selected_blocks.insert(file_id.clone(),self.active_block.clone());}
                            self.editor_key.clear();self.sync_editor();self.schedule_render(false);}}}
                        // Deleted and written back unchanged (an editor's atomic save): it is here again.
                        else if buffer.external.as_ref().is_some_and(|external|external.deleted){buffer.external=None;}}}
                    else {let recover=document.recovery.is_some();let external=document.disk_missing.then_some(ExternalVersion{content:None,hash:None,deleted:true});self.buffers.insert(file_id.clone(),Buffer{base_content:document.content.clone(),document,revision:0,saving:None,external,last_used:Instant::now()});if intent.activate{self.activate(file_id.clone(),intent.block,intent.record);}if recover{self.recovery_file=Some(file_id);}}
                },Err(reason)=>self.notify(reason)}
            },
            CoreEvent::Saved{file_id,result}=>{let saved_content=self.buffers.get_mut(&file_id).and_then(|buffer|buffer.saving.take());match result{SaveResult::Saved{path,disk_hash,copy}=>{if copy{self.notify(format!("Saved a copy to {}",path.display()));}else if let Some(buffer)=self.buffers.get_mut(&file_id){buffer.base_content=saved_content.unwrap_or_else(||buffer.document.content.clone());buffer.document.disk_hash=disk_hash;buffer.external=None;buffer.document.recovery=None;self.recoveries.retain(|record|record.file_id!=file_id);if !buffer.dirty(){if let Some(core)=self.core.as_mut(){let _=core.discard_recovery(&file_id);}}else{self.last_journal=Some(Instant::now());}self.notify("File saved");}},SaveResult::Conflict{message,disk_content,disk_hash}=>{if let Some(buffer)=self.buffers.get_mut(&file_id){buffer.external=Some(ExternalVersion{content:disk_content,hash:disk_hash,deleted:false});}self.conflict_file=Some(file_id);self.notify(message);},SaveResult::Failed(reason)=>self.notify(reason)}self.prune_buffers();},
            CoreEvent::External{file_id,kind,disk_hash}=>{if let Some(buffer)=self.buffers.get_mut(&file_id){if disk_hash.as_ref()==Some(&buffer.document.disk_hash){continue;}if kind==ChangeKind::Deleted{buffer.external=Some(ExternalVersion{content:None,hash:None,deleted:true});self.notify("A file was moved or deleted. Its buffer is preserved.");}else{self.reads.insert(file_id.clone(),ReadIntent{generation:self.generation,activate:false,block:None,record:false,external:true});if let Some(core)=self.core.as_mut(){core.request_read(file_id);}}}},
            CoreEvent::Error(reason)=>self.notify(reason),
        }}
        for event in self.renderer.poll(){match event{
            RenderEvent::Completed{job_id,diagram_id,result}=>{
                // Accept only the result of the active job for the active diagram.
                let wanted=self.block().map(|block|block.id.clone());
                if wanted.as_deref()!=Some(diagram_id.as_str())||!self.active_job.as_ref().is_some_and(|(id,_,_)|*id==job_id){continue;}
                let (_,key,signature)=self.active_job.take().unwrap();
                match result{Ok(artifact)=>self.offer_artifact(key,artifact,signature),Err(diagnostic)=>{if diagnostic.kind!=DiagnosticKind::Canceled{self.diagnostics.insert(key,diagnostic);}}}
            },
            RenderEvent::Exported{job_id,result}=>{
                if self.export.job==Some(job_id){self.export.job=None;}
                match result{Ok(path)=>self.notify(format!("Exported to {}",path.display())),Err(reason)=>self.notify(reason)}
            },
        }}
        self.release_held(ctx);
        if self.prefetch_due&&self.pending_render.is_none()&&self.active_job.is_none()&&self.held.is_none(){
            if self.renderer.is_busy(){ctx.request_repaint_after(Duration::from_millis(40));}else{self.prefetch_due=false;self.prefetch_neighbours();}
        }
        if self.pending_render.as_ref().is_some_and(|pending|Instant::now()>=pending.at){let pending=self.pending_render.take().unwrap();self.active_job=Some((pending.request.job_id,pending.key,pending.signature));self.renderer.render(pending.request);}
        if let Some(pending)=&self.pending_render{ctx.request_repaint_after(pending.at.saturating_duration_since(Instant::now()));}
        if self.last_journal.is_some_and(|time|time.elapsed()>Duration::from_millis(250)){if let Err(reason)=self.journal_all(){self.notify(format!("Recovery could not be saved: {reason}"));}}
        if self.shot.is_none()&&self.last_persist.is_some_and(|time|time.elapsed()>Duration::from_millis(400)){if let Some(core)=self.core.as_mut(){let _=core.save_workspace_state(self.state.clone());}self.last_persist=None;}
        if self.last_journal.is_some()||self.last_persist.is_some(){ctx.request_repaint_after(Duration::from_millis(250));}
        if self.settings.autosave&&self.last_journal.is_none(){let ids=self.buffers.iter().filter(|(_,buffer)|buffer.dirty()&&buffer.saving.is_none()&&buffer.external.is_none()&&!buffer.document.read_only&&buffer.last_used.elapsed()>Duration::from_millis(1200)).map(|(id,_)|id.clone()).collect::<Vec<_>>();for id in ids{self.save(id,false);}}
        if self.settings.autosave&&self.buffers.values().any(|buffer|buffer.dirty()&&buffer.external.is_none()){ctx.request_repaint_after(Duration::from_millis(400));}
        if self.toast.as_ref().is_some_and(|(_,time)|time.elapsed()>TOAST_LIFETIME){self.toast=None;}
    }

    /// Moves the view to the next search hit (wrapping) and selects it.
    pub(crate) fn focus_next_match(&mut self){
        let Some(artifact)=self.active_preview().map(|preview|preview.artifact.clone())else{return;};
        if self.search_hits.is_empty(){return;}
        let next=self.search_cursor.map_or(0,|index|(index+1)%self.search_hits.len());
        self.search_cursor=Some(next);self.canvas.focus_node(&artifact.rendered.scene,self.search_hits[next]);
    }
    /// Recomputes search hits when the query or the drawn artifact changes.
    pub(crate) fn update_search_hits(&mut self,artifact:Option<&Arc<RenderArtifact>>){
        let query=if self.search_open{self.diagram_search.trim().to_owned()}else{String::new()};
        let id=artifact.map(|artifact|artifact.id.clone()).unwrap_or_default();
        if self.search_key.0==id&&self.search_key.1==query{return;}
        self.search_hits=artifact.map(|artifact|artifact.rendered.scene.find(&query)).unwrap_or_default();
        self.canvas.matches.clone_from(&self.search_hits);self.search_cursor=None;self.search_key=(id,query);
    }
    /// Title and body shown instead of the canvas for unsupported families and engine failures.
    pub(crate) fn canvas_message(&self,artifact:Option<&Arc<RenderArtifact>>,key:&str)->Option<(String,String,CardAction)>{
        if let Some(artifact)=artifact{
            let title=match artifact.rendered.family{
                Family::Flowchart=>return None,
                Family::Sequence=>"Sequence diagrams are not supported yet",
                Family::State=>"State diagrams are not supported yet",
                Family::Class=>"Class diagrams are not supported yet",
                Family::Er=>"ER diagrams are not supported yet",
                Family::Other=>"This diagram type is not supported yet",
            };
            return Some((title.into(),"Flowcharts render today. The source is still here to read and edit.".into(),CardAction::OpenSource));
        }
        let diagnostic=self.diagnostics.get(key)?;
        match diagnostic.kind{
            DiagnosticKind::Budget=>Some(("This diagram is too large to draw".into(),diagnostic.message.clone(),CardAction::OpenSource)),
            DiagnosticKind::Crash=>Some(("Something went wrong drawing this diagram".into(),diagnostic.message.clone(),CardAction::Retry)),
            _=>None,
        }
    }
    /// Selects `span` of the active diagram in the open editor, when the picture matches the source.
    pub(crate) fn select_in_source(&mut self,span:Span)->bool{
        if !self.editor_open{return false;}
        let range=self.block().zip(self.buffers.get(&self.active_file)).filter(|(block,_)|self.active_preview().is_some_and(|preview|preview.signature==block.source)).and_then(|(block,buffer)|source_map::block_to_editor_chars(block,self.full_file,&buffer.document.content,span));
        if let Some((start,end))=range{self.editor.select_range((start,end));self.cursor_seen=Some(end);true}else{false}
    }
    /// Highlights the node under the editor cursor, without moving the camera.
    pub(crate) fn select_from_cursor(&mut self,cursor:usize){
        let node=self.block().zip(self.buffers.get(&self.active_file)).zip(self.active_preview()).filter(|((block,_),preview)|preview.signature==block.source).and_then(|((block,buffer),preview)|{
            let scene=&preview.artifact.rendered.scene;
            let at=|chars:usize|source_map::editor_char_to_block_byte(block,self.full_file,&buffer.document.content,chars).and_then(|byte|scene.node_at_offset(byte));
            at(cursor).or_else(||cursor.checked_sub(1).and_then(at))
        });
        match node{Some(node)=>{self.canvas.selected=Some(node);self.canvas.selected_edge=None;},None=>{self.canvas.clear_selection();}}
    }
    /// Why the active diagram cannot be exported, if it cannot.
    pub(crate) fn export_unavailable(&self)->Option<&'static str>{
        let Some(preview)=self.active_preview()else{return Some("Nothing is drawn yet")};
        if preview.artifact.rendered.family!=Family::Flowchart{return Some("Only flowcharts can be exported yet");}
        if preview.artifact.rendered.scene.nodes.is_empty(){return Some("This diagram has no nodes to export");}
        None
    }
    /// Asks for a file name and hands the export to the engine.
    pub(crate) fn start_export(&mut self){
        let Some(artifact)=self.active_preview().map(|preview|preview.artifact.clone())else{return;};
        let (extension,label)=match self.export.format{ExportFormat::Svg=>("svg","SVG image"),ExportFormat::Png=>("png","PNG image")};
        let stem=self.buffers.get(&self.active_file).map(|buffer|{let name=buffer.document.name.rsplit_once('.').map_or(buffer.document.name.as_str(),|(stem,_)|stem).to_owned();match buffer.document.blocks.iter().position(|block|block.id==self.active_block){Some(index) if buffer.document.blocks.len()>1=>format!("{name}-{}",index+1),_=>name}}).unwrap_or_else(||"diagram".into());
        let Some(path)=rfd::FileDialog::new().set_title("Export diagram").add_filter(label,&[extension]).set_file_name(format!("{stem}.{extension}")).save_file()else{return;};
        self.submit_export(artifact,path);
    }
    pub(crate) fn submit_export(&mut self,artifact:Arc<RenderArtifact>,path:PathBuf){
        let job_id=self.next_job;self.next_job+=1;self.export.job=Some(job_id);
        let scale=if self.export.format==ExportFormat::Png{self.export.scale as f32}else{1.0};
        self.renderer.export(ExportRequest{job_id,artifact,path,format:self.export.format,background:self.export.background,scale,dark:self.dark});
        self.dialog=None;
    }
    /// Dev hook for screenshots: activates the requested file of the opened folder once it is indexed.
    fn drive_shot_open(&mut self){
        let Some(wanted)=self.shot_open.clone()else{return;};
        let Some(workspace)=&self.workspace else{return;};
        if workspace.indexing{return;}
        let wanted=std::fs::canonicalize(&wanted).unwrap_or(wanted);
        let found=workspace.files.iter().find(|file|std::fs::canonicalize(workspace.root.join(&file.path)).is_ok_and(|path|path==wanted)).map(|file|file.id.clone());
        self.shot_open=None;
        if let Some(id)=found{self.open_document(id,None,false);}else{self.notify("Screenshot file is not in the opened folder");}
    }
    pub(crate) fn open_diagnostics(&mut self){
        self.dialog=Some(Dialog::Diagnostics);let artifact=self.active_preview().map(|preview|preview.artifact.clone());
        let engine=self.renderer.diagnostics();
        self.diagnostics_text=format!("Tidemap {}\nRuntime: native Rust + egui / eframe\nProtocol: {}\nFiles indexed: {}\nOpen buffers: {}\nDirty buffers: {}\nCached artifacts: {}\nActive renderer job: {}\nSource offsets: UTF-8 bytes\nPersistence: native app-data journals\n\nActive preview\nEngine: tidemap-diagram, in process\nRender time: {:.1} ms\nBytes: {}\nNodes: {}\nEdges: {}\nEngine cache: {}\nPainted last frame: {}\n\nNo benchmark or release validation has been performed.",env!("CARGO_PKG_VERSION"),PROTOCOL_VERSION,self.workspace.as_ref().map(|workspace|workspace.files.len()).unwrap_or(0),self.buffers.len(),self.buffers.values().filter(|buffer|buffer.dirty()).count(),self.previews.len(),self.active_job.is_some(),artifact.as_ref().map(|artifact|artifact.duration_us as f64/1000.0).unwrap_or(0.0),artifact.as_ref().map(|artifact|artifact.bytes).unwrap_or(0),artifact.as_ref().map(|artifact|artifact.rendered.scene.nodes.len()).unwrap_or(0),artifact.as_ref().map(|artifact|artifact.rendered.scene.edges.len()).unwrap_or(0),engine,self.canvas.last_stats.map(|stats|format!("{} items, {} texts, {:?} detail",stats.items,stats.texts,stats.lod)).unwrap_or_else(||"nothing".into()));
    }
}

/// What the recovery dialog shows about a file with an unsaved recovery copy.
pub(crate) struct RecoveryInfo {
    pub name: String,
    /// The file on disk changed since the recovery copy was made.
    pub disk_changed: bool,
}

/// What the conflict dialog compares.
pub(crate) struct ConflictInfo {
    pub name: String,
    pub disk_hash: Option<String>,
    pub ours: String,
    pub theirs: String,
    pub deleted: bool,
}

/// Commands: everything the palette, the shortcuts and the dialogs ask the app to do.
impl TidemapApp {
    pub(crate) fn run_command(&mut self, command: Command) {
        let file = self.active_file.clone();
        match command {
            Command::Save => self.save(file, false),
            Command::SaveCopy => self.save(file, true),
            Command::ToggleSource => {
                self.editor_open = !self.editor_open;
                self.record_state();
            }
            Command::ToggleSidebar => self.sidebar_open = !self.sidebar_open,
            Command::Fit => self.canvas.fit(),
            Command::ActualSize => self.canvas.actual_size(),
            Command::ExportDiagram => match self.export_unavailable() {
                Some(reason) => self.notify(reason),
                None => self.dialog = Some(Dialog::Export),
            },
            Command::ToggleTheme => {
                let from = if self.theme_anchor.is_positive() { self.theme_anchor.center() } else { self.ctx.content_rect().right_top() };
                self.request_theme_toggle(from);
            }
            Command::OpenFolder => self.open_folder(None),
            Command::OpenFile => self.open_external(None),
            Command::NewDiagram => self.new_diagram(),
            Command::PinFile => self.toggle_pin(),
            Command::NextDiagram => self.step_diagram(1),
            Command::PreviousDiagram => self.step_diagram(-1),
            Command::FindInDiagram => self.open_search(),
            Command::Settings => self.dialog = Some(Dialog::Settings),
            Command::Shortcuts => self.dialog = Some(Dialog::Help),
            Command::ZoomIn => self.canvas.zoom_by(1.25),
            Command::ZoomOut => self.canvas.zoom_by(0.8),
            Command::SwitchProject => self.toggle_projects(),
            Command::PreviousFile => self.previous_file(),
            Command::FocusFiles => self.focus_sidebar(ui::sidebar::FocusTarget::Tree),
            Command::SearchFiles => self.focus_sidebar(ui::sidebar::FocusTarget::Search),
            Command::ToggleOutline => self.outline_open = !self.outline_open,
            Command::ToggleDocument => self.toggle_document_view(),
        }
    }

    /// Opens the project switcher, or closes it when it is already showing.
    pub(crate) fn toggle_projects(&mut self) {
        if self.palette_open && self.palette_mode == ui::palette::PaletteMode::Projects {
            self.palette_open = false;
            return;
        }
        self.palette_open = true;
        self.palette_mode = ui::palette::PaletteMode::Projects;
        self.palette_query.clear();
        self.palette_index = 0;
        self.palette_fresh = true;
        self.info_open = false;
    }

    /// Opens a recent project. The current one is journaled first, so nothing is lost.
    pub(crate) fn switch_project(&mut self, root: PathBuf) {
        if self.workspace.as_ref().is_some_and(|workspace| workspace.root == root) {
            return;
        }
        if !root.is_dir() {
            self.notify(format!("{} is missing. Press Delete in the project list to remove it.", root.display()));
            return;
        }
        self.open_folder(Some(root));
    }

    /// Drops a project from the recent list (the folder itself is untouched).
    pub(crate) fn forget_project(&mut self, root: &std::path::Path, name: &str) {
        let Some(core) = self.core.as_mut() else { return };
        match core.forget_workspace(root) {
            Ok(()) => {
                self.recents.retain(|recent| recent.root != root);
                self.notify(format!("Removed {name} from recent projects"));
            }
            Err(reason) => self.notify(reason.to_string()),
        }
    }

    /// Shows the sidebar and puts the keyboard in its search field or its tree.
    pub(crate) fn focus_sidebar(&mut self, target: ui::sidebar::FocusTarget) {
        if self.workspace.is_none() {
            self.notify("Open a folder to browse its files");
            return;
        }
        self.sidebar_open = true;
        self.focus_request = Some(target);
    }

    /// Goes back to the most recently viewed other file (Ctrl+Tab).
    pub(crate) fn previous_file(&mut self) {
        // Walk back from the current place in history first, then forward.
        let upto = self.state.history.len().min(self.state.history_index + 1);
        let entry = self.state.history[..upto]
            .iter()
            .rev()
            .chain(self.state.history[upto..].iter())
            .find(|entry| entry.file_id != self.active_file)
            .cloned();
        match entry {
            Some(entry) => self.open_document(entry.file_id, Some(entry.block_id), true),
            None => self.notify("No other file opened yet"),
        }
    }

    fn toggle_pin(&mut self) {
        if self.active_file.is_empty() {
            return;
        }
        match self.state.pins.iter().position(|pin| pin == &self.active_file) {
            Some(at) => {
                self.state.pins.remove(at);
            }
            None => self.state.pins.push(self.active_file.clone()),
        }
        self.record_state();
    }

    /// Opens the palette with `prefill` typed in, or closes it when it is open and `prefill` is empty.
    pub(crate) fn toggle_palette(&mut self, prefill: &str) {
        if self.palette_open && prefill.is_empty() {
            self.palette_open = false;
            return;
        }
        self.palette_open = true;
        self.palette_mode = ui::palette::PaletteMode::Files;
        self.palette_query = prefill.to_owned();
        self.palette_index = 0;
        self.palette_fresh = true;
        self.info_open = false;
    }

    pub(crate) fn open_search(&mut self) {
        if self.document_view() {
            // The page has no nodes to search: find text on the page itself.
            self.docview.open_find();
            return;
        }
        self.search_open = true;
        self.search_fresh = true;
    }

    pub(crate) fn close_search(&mut self) {
        self.search_open = false;
        self.diagram_search.clear();
    }

    pub(crate) fn show_source(&mut self) {
        self.editor_open = true;
        self.record_state();
    }

    /// Opens the source panel on the whole file.
    pub(crate) fn show_full_source(&mut self) {
        self.full_file = true;
        self.editor_open = true;
        self.editor_key.clear();
        self.record_state();
    }

    /// Opens the source at a problem: selects its span when the picture matches the source, else jumps to its line.
    pub(crate) fn show_problem(&mut self, line: usize, span: Option<Span>) {
        self.show_source();
        if let Some(span) = span {
            if self.select_in_source(span) {
                return;
            }
        }
        let offset = if self.full_file {
            self.block().map_or(0, |block| block.start_line.saturating_sub(1))
        } else {
            0
        };
        self.editor.jump_to_line(line + offset);
    }

    pub(crate) fn cancel_render(&mut self) {
        self.renderer.cancel();
        self.pending_render = None;
        self.active_job = None;
        self.notify("Drawing canceled. Your source is kept.");
    }

    /// The layout whose diagnostics the info popover lists: a held one first, else the drawn one.
    pub(crate) fn notes_artifact(&self) -> Option<Arc<RenderArtifact>> {
        let key = self.diagram_key();
        self.held
            .as_ref()
            .filter(|held| held.key == key)
            .map(|held| held.artifact.clone())
            .or_else(|| self.previews.get(&key).map(|preview| preview.artifact.clone()))
    }

    /// Whether the picture shown is older than the source because the newest layout has problems.
    pub(crate) fn held_for_active(&self) -> bool {
        self.held.as_ref().is_some_and(|held| held.key == self.diagram_key())
    }

    /// Problems in the active diagram: syntax diagnostics plus a failed layout.
    pub(crate) fn problem_count(&self) -> usize {
        let listed = self.notes_artifact().map_or(0, |artifact| artifact.rendered.diagnostics.len());
        listed + usize::from(self.diagnostics.contains_key(&self.diagram_key()))
    }

    /// Files with unsaved work, counting recovery copies of files that are not open.
    pub(crate) fn unsaved_count(&self) -> usize {
        let recoveries = self.core.as_ref().map_or(0, |core| core.active_recoveries().len());
        recoveries.max(self.buffers.values().filter(|buffer| buffer.dirty()).count()).max(1)
    }

    /// Applies what the Diagram style group of the settings dialog asked for.
    pub(crate) fn apply_style_choices(&mut self, color_by: ColorBy, open_file: bool, ask_reset: bool, keep: bool, reset: bool) {
        if color_by != self.style_file.sheet.color_by {
            let mut sheet = self.style_file.sheet.clone();
            sheet.color_by = color_by;
            match self.style_file.save(&sheet) {
                Ok(()) => self.apply_style(),
                Err(reason) => self.notify(reason),
            }
        }
        if open_file {
            if let Err(reason) = style_file::open_in_default_app(&self.style_file.path) {
                self.notify(reason);
            }
        }
        if ask_reset {
            self.style_confirm = true;
        }
        if keep {
            self.style_confirm = false;
        }
        if reset {
            self.style_confirm = false;
            match self.style_file.reset() {
                Ok(()) => self.apply_style(),
                Err(reason) => self.notify(reason),
            }
        }
    }

    pub(crate) fn recovery_info(&self, id: &str) -> Option<RecoveryInfo> {
        let buffer = self.buffers.get(id)?;
        let record = buffer.document.recovery.as_ref()?;
        Some(RecoveryInfo {
            name: buffer.document.name.clone(),
            disk_changed: buffer.document.disk_hash != record.base_hash,
        })
    }

    /// Puts the recovery copy into the buffer; a newer file on disk is kept as the external version.
    pub(crate) fn recover_changes(&mut self, id: String) {
        let Some(record) = self.buffers.get(&id).and_then(|buffer| buffer.document.recovery.clone()) else {
            self.recovery_file = None;
            return;
        };
        if let Some(buffer) = self.buffers.get_mut(&id) {
            let previous = buffer.document.blocks.clone();
            if buffer.document.disk_hash != record.base_hash {
                buffer.external = Some(ExternalVersion {
                    content: Some(buffer.base_content.clone()),
                    hash: Some(buffer.document.disk_hash.clone()),
                    deleted: false,
                });
                buffer.document.disk_hash = record.base_hash;
            }
            buffer.document.content = record.content;
            buffer.document.blocks = extract_diagrams(&buffer.document.content, &buffer.document.path, &previous);
            buffer.document.recovery = None;
            buffer.revision += 1;
        }
        self.recovery_file = None;
        self.activate(id, None, true);
        self.last_journal = Some(Instant::now());
        self.notify("Recovered unsaved changes");
    }

    /// Drops the recovery copy and keeps the file as it is on disk.
    pub(crate) fn use_disk_version(&mut self, id: String) {
        if let Some(core) = self.core.as_mut() {
            if let Err(reason) = core.discard_recovery(&id) {
                self.notify(reason.to_string());
                return;
            }
        }
        if let Some(buffer) = self.buffers.get_mut(&id) {
            buffer.document.recovery = None;
        }
        self.recoveries.retain(|record| record.file_id != id);
        self.recovery_file = None;
    }

    pub(crate) fn conflict_info(&self, id: &str) -> Option<ConflictInfo> {
        let buffer = self.buffers.get(id)?;
        let external = buffer.external.as_ref();
        Some(ConflictInfo {
            name: buffer.document.name.clone(),
            disk_hash: external.and_then(|external| external.hash.clone()),
            ours: buffer.document.content.clone(),
            theirs: external
                .and_then(|external| external.content.clone())
                .unwrap_or_else(|| "The file is unavailable or read-only. Save your source as a copy.".into()),
            deleted: external.is_some_and(|external| external.deleted),
        })
    }

    /// Throws away every recovery copy and closes the window.
    pub(crate) fn discard_and_close(&mut self, ctx: &egui::Context) {
        if let Some(core) = self.core.as_mut() {
            for record in core.active_recoveries() {
                let _ = core.discard_recovery(&record.file_id);
            }
            for id in self.buffers.keys() {
                let _ = core.discard_recovery(id);
            }
            let _ = core.flush();
        }
        self.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Saves every file with unsaved work, then closes the window. A conflict stops it and opens the conflict dialog.
    pub(crate) fn save_and_close(&mut self, ctx: &egui::Context) {
        self.record_state();
        if let Err(reason) = self.journal_all() {
            self.notify(reason);
            return;
        }
        let mut dirty: Vec<(String, String, String)> = self
            .buffers
            .iter()
            .filter(|(_, buffer)| buffer.dirty())
            .map(|(id, buffer)| (id.clone(), buffer.document.content.clone(), buffer.document.disk_hash.clone()))
            .collect();
        if let Some(core) = self.core.as_ref() {
            for record in core.active_recoveries() {
                if !dirty.iter().any(|(id, _, _)| id == &record.file_id) {
                    dirty.push((record.file_id, record.content, record.base_hash));
                }
            }
        }
        for (id, content, expected_hash) in dirty {
            let blocked = self
                .buffers
                .get(&id)
                .is_some_and(|buffer| buffer.external.is_some() || buffer.document.read_only);
            if blocked {
                self.dialog = None;
                self.conflict_file = Some(id);
                return;
            }
            let request = SaveRequest { file_id: id.clone(), content: content.clone(), expected_hash };
            let result = self.core.as_mut().map(|core| core.save_now(request));
            match result {
                Some(SaveResult::Saved { disk_hash, .. }) => {
                    if let Some(buffer) = self.buffers.get_mut(&id) {
                        buffer.base_content = content;
                        buffer.document.disk_hash = disk_hash;
                        buffer.document.recovery = None;
                    }
                    if let Some(core) = self.core.as_mut() {
                        let _ = core.discard_recovery(&id);
                    }
                }
                Some(SaveResult::Conflict { message, disk_content, disk_hash }) => {
                    if let Some(buffer) = self.buffers.get_mut(&id) {
                        buffer.external = Some(ExternalVersion { content: disk_content, hash: disk_hash, deleted: false });
                        self.conflict_file = Some(id);
                        self.dialog = None;
                    }
                    self.notify(format!("{message} Recovery is preserved. Reopen that file to review its conflict."));
                    return;
                }
                Some(SaveResult::Failed(reason)) => {
                    self.notify(reason);
                    return;
                }
                None => return,
            }
        }
        if let Some(core) = self.core.as_mut() {
            if let Err(reason) = core.flush() {
                self.notify(reason.to_string());
                return;
            }
        }
        self.allow_close = true;
        self.renderer.cancel();
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// The window asked to close: ask first when there is unsaved work.
    fn request_close(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.record_state();
        if let Err(reason) = self.journal_all() {
            self.notify(reason);
        }
        let unsaved = self.buffers.values().any(|buffer| buffer.dirty() || buffer.saving.is_some())
            || self.core.as_ref().is_some_and(|core| !core.active_recoveries().is_empty());
        if unsaved {
            self.dialog = Some(Dialog::Close);
        } else {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Screenshot dev mode: surfaces are opened by hooks, then the window is saved as a PNG and the app quits.
impl TidemapApp {
    /// Whether a requested hook has not run yet.
    fn shot_hooks_pending(&self) -> bool {
        self.shot.as_ref().is_some_and(|shot| {
            shot.select.is_some()
                || shot.dialog.is_some()
                || shot.info
                || shot.palette.is_some()
                || shot.search.is_some()
                || shot.outline
        })
    }

    /// Opens what the screenshot hooks ask for, once the folder is indexed and the diagram is drawn.
    pub(crate) fn drive_shot_hooks(&mut self) {
        if !self.shot_hooks_pending() {
            return;
        }
        let indexing = self.workspace.as_ref().is_some_and(|workspace| workspace.indexing);
        if indexing || self.shot_open.is_some() || !self.reads.is_empty() {
            return;
        }
        let Some(shot) = self.shot.as_ref() else { return };
        let needs_diagram = self.block().is_some()
            && (shot.select.is_some()
                || matches!(shot.dialog.as_deref(), Some("export" | "diagnostics"))
                || shot.info
                || shot.search.is_some()
                || shot.outline);
        let artifact = self.active_preview().map(|preview| preview.artifact.clone());
        if needs_diagram && artifact.is_none() && !self.diagnostics.contains_key(&self.diagram_key()) {
            return;
        }
        let Some(shot) = self.shot.as_mut() else { return };
        let select = shot.select.take();
        let dialog = shot.dialog.take();
        let info = std::mem::take(&mut shot.info);
        let palette = shot.palette.take();
        let search = shot.search.take();
        let outline = std::mem::take(&mut shot.outline);
        shot.frames = 0;

        if let (Some(text), Some(artifact)) = (select, &artifact) {
            let needle = text.to_lowercase();
            let found = artifact
                .rendered
                .scene
                .nodes
                .iter()
                .find(|node| node.text.lines.join(" ").to_lowercase().contains(&needle))
                .map(|node| (node.id, node.span));
            if let Some((id, span)) = found {
                self.canvas.selected = Some(id);
                self.select_in_source(span);
            }
        }
        match dialog.as_deref() {
            Some("export") => self.dialog = Some(Dialog::Export),
            Some("settings") => self.dialog = Some(Dialog::Settings),
            Some("help") => self.dialog = Some(Dialog::Help),
            Some("close") => self.dialog = Some(Dialog::Close),
            Some("diagnostics") => self.open_diagnostics(),
            Some("conflict") => self.shot_conflict(),
            _ => {}
        }
        self.info_open = info;
        if let Some(query) = palette {
            // `projects:` opens the project switcher (with the rest as its query).
            if let Some(rest) = query.strip_prefix("projects:") {
                self.toggle_projects();
                self.palette_query = rest.to_owned();
            } else {
                self.toggle_palette(&query);
            }
        }
        if let Some(text) = search {
            self.open_search();
            self.diagram_search = text;
        }
        self.outline_open |= outline;
        self.canvas.painted_frames = 0;
    }

    /// Dev hook: pretends the open file changed on disk, so the conflict dialog can be shot.
    fn shot_conflict(&mut self) {
        let id = self.active_file.clone();
        if let Some(buffer) = self.buffers.get_mut(&id) {
            let theirs = format!("{}
  Extra[Added elsewhere] --> Start", buffer.document.content.trim_end());
            buffer.external = Some(ExternalVersion { content: Some(theirs), hash: Some("0f3a9c1d7e5b2468".into()), deleted: false });
            self.conflict_file = Some(id);
        }
    }

    /// Once the surface has settled, saves the window as a PNG and quits.
    pub(crate) fn drive_screenshot(&mut self, ctx: &egui::Context) {
        let hooks_pending = self.shot_hooks_pending();
        let Some(shot) = self.shot.as_mut() else { return };
        shot.frames += 1;
        let (path, requested, started, frames) = (shot.path.clone(), shot.requested, shot.started, shot.frames);
        ctx.request_repaint_after(Duration::from_millis(30));
        let image = ctx.input(|input| {
            input.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, user_data, .. } if user_data.data.is_none() => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
            if let Err(reason) =
                image::save_buffer(&path, &bytes, image.width() as u32, image.height() as u32, image::ColorType::Rgba8)
            {
                eprintln!("Screenshot: {reason}");
            }
            // Bypass the unsaved-changes dialog.
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if requested {
            return;
        }
        let static_view = self.block().is_none() && self.reads.is_empty();
        let drawn = self.canvas.painted_frames >= SHOT_STABLE_FRAMES
            || self.card_frames >= SHOT_STABLE_FRAMES
            || (static_view && frames >= SHOT_STATIC_FRAMES);
        let indexing = self.workspace.as_ref().is_some_and(|workspace| workspace.indexing);
        let settled = drawn
            && !hooks_pending
            && !indexing
            && self.shot_open.is_none()
            && self.held.is_none()
            && !self.canvas.is_animating()
            && !crate::motion::busy(ctx)
            && self.pending_render.is_none()
            && self.active_job.is_none()
            && !self.renderer.is_busy();
        if settled || started.elapsed() > SHOT_TIMEOUT {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            if let Some(shot) = self.shot.as_mut() {
                shot.requested = true;
            }
        }
    }
}


impl TidemapApp {
    /// Whether the center shows the rendered Markdown page instead of the diagram canvas.
    pub(crate) fn document_view(&self) -> bool {
        !self.state.diagram_view
            && self.buffers.get(&self.active_file).is_some_and(|buffer| !buffer.document.name.to_lowercase().ends_with(".mmd"))
    }

    pub(crate) fn toggle_document_view(&mut self) {
        let markdown = self.buffers.get(&self.active_file).is_some_and(|buffer| !buffer.document.name.to_lowercase().ends_with(".mmd"));
        if !markdown {
            self.notify("Only Markdown files have a page view");
            return;
        }
        self.request_page_switch(None);
    }

    /// Switches between the page and the diagram once the old frame is snapshotted. `embed` is
    /// the diagram to open and where it sits on screen, when the switch is a click on an embed.
    /// Each request is answered by exactly one page purpose, so `page_opens` stays in step.
    fn request_page_switch(&mut self, embed: Option<(String, egui::Rect)>) {
        self.page_opens.push_back(embed);
        ui::snapshot::request(&self.ctx, ui::snapshot::Purpose::Page { clip: self.canvas_area });
    }

    /// Switches the appearance between light and dark, spreading from `from` (a screen point).
    /// Every theme toggle (button, shortcut, palette) comes through here.
    pub(crate) fn request_theme_toggle(&mut self, from: egui::Pos2) {
        ui::snapshot::request(&self.ctx, ui::snapshot::Purpose::Theme { from });
    }

    /// Makes the change a snapshot was taken for, in the frame the screenshot arrives.
    fn apply_snapshot(&mut self, purpose: ui::snapshot::Purpose) {
        match purpose {
            ui::snapshot::Purpose::Theme { .. } => {
                self.settings.theme = ui::snapshot::toggled_theme(self.settings.theme, self.dark);
                self.save_settings();
            }
            ui::snapshot::Purpose::Page { .. } => {
                match self.page_opens.pop_front().flatten() {
                    Some((id, from)) => {
                        self.select_block(id);
                        self.canvas.enter_from(&self.ctx, &self.active_block, from);
                        self.state.diagram_view = true;
                    }
                    None => self.state.diagram_view = !self.state.diagram_view,
                }
                self.record_state();
            }
        }
    }

    /// Folder of the active document on disk. Workspace paths are stored relative to the root.
    pub(crate) fn document_dir(&self) -> Option<std::path::PathBuf> {
        let buffer = self.buffers.get(&self.active_file)?;
        let path = std::path::Path::new(&buffer.document.path);
        let full = match (&self.workspace, path.is_absolute()) {
            (Some(workspace), false) => workspace.root.join(path),
            _ => path.to_path_buf(),
        };
        full.parent().map(std::path::Path::to_path_buf)
    }

    /// Paints the rendered page of the active Markdown file and carries out what it asks for.
    pub(crate) fn show_document(&mut self, ui: &mut egui::Ui, p: theme::Palette) {
        use crate::markdown::view::{Action, Input};
        let key = format!("doc:{}", self.active_file);
        let saved_scroll = self.state.views.get(&key).map_or(0.0, |view| view.scroll);
        let dir = self.document_dir();
        let Some(buffer) = self.buffers.get(&self.active_file) else { return };
        let input = Input {
            file_id: &self.active_file,
            content: &buffer.document.content,
            dir: dir.as_deref(),
            revision: buffer.revision,
            blocks: &buffer.document.blocks,
            active_block: &self.active_block,
            p,
            dark: self.dark,
            style: &self.style,
            saved_scroll,
        };
        let output = self.docview.show(ui, &input, &mut self.renderer, &mut self.next_job);
        let view = self.state.views.entry(key).or_default();
        view.scroll = output.scroll;
        for action in output.actions {
            match action {
                Action::SelectDiagram(id) => self.select_block(id),
                Action::OpenDiagram(id, from) => self.request_page_switch(Some((id, from))),
                Action::OpenUrl(url) => ui.ctx().open_url(egui::OpenUrl::new_tab(url)),
                Action::OpenPath(link) => {
                    let target = link.split('#').next().unwrap_or("");
                    if target.is_empty() {
                        continue;
                    }
                    let base = dir.clone().unwrap_or_default();
                    let full = base.join(target.replace('/', std::path::MAIN_SEPARATOR_STR));
                    if full.is_file() {
                        self.open_external(Some(full));
                    } else {
                        self.notify(format!("Cannot find {target}"));
                    }
                }
                Action::EditLine(line) => {
                    self.show_full_source();
                    self.editor.jump_to_line(line);
                }
                Action::Copied => self.notify("Copied"),
            }
        }
    }
}

impl eframe::App for TidemapApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::from(theme::palette(self.dark).app).to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        static SLOWMO: std::sync::Once = std::sync::Once::new();
        SLOWMO.call_once(|| {
            // `TIDEMAP_SLOWMO=4` plays every animation four times slower; junk means 1.
            let scale = std::env::var("TIDEMAP_SLOWMO")
                .ok()
                .and_then(|value| value.trim().parse::<f32>().ok())
                .unwrap_or(1.0);
            crate::motion::set_time_scale(ctx, scale);
        });
        for purpose in ui::snapshot::take(ctx) {
            self.apply_snapshot(purpose);
        }
        self.dark = theme::apply(ctx, self.settings.theme);
        self.font_ready = true;
        self.poll(ctx);
        self.restore_pending();
        ui::palette::shortcuts(self, ctx);
        let dropped: Vec<PathBuf> =
            ctx.input(|input| input.raw.dropped_files.iter().filter_map(|file| file.path.clone()).collect());
        for path in dropped {
            if path.is_dir() {
                self.open_folder(Some(path));
            } else {
                self.open_external(Some(path));
            }
        }
        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_close {
            self.request_close(ctx);
        }
        ui::frame(self, ctx);
        if self.active_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.drive_shot_open();
        self.drive_shot_hooks();
        self.drive_screenshot(ctx);
        ui::snapshot::paint(ctx);
    }
}
