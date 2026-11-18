use egui::{Color32, FontId, RichText, TextFormat};
use crate::theme::{Palette, token};

#[derive(Default)]
pub struct SourceEditor {
    pub find_open: bool,
    pub find: String,
    jump_line: Option<usize>,
    select: Option<(usize, usize)>,
}

pub struct EditorResponse { pub changed: bool, pub cursor: usize, pub scroll: f32, pub focused: bool }

impl SourceEditor {
    pub fn find(&mut self) { self.find_open = true; }
    pub fn jump_to_line(&mut self, line: usize) { self.jump_line = Some(line.max(1)); }
    /// Selects the char range `chars` and scrolls it into view on the next `show`.
    pub fn select_range(&mut self, chars: (usize, usize)) { self.select = Some(chars); }

    pub fn show(&mut self, ui: &mut egui::Ui, key: &str, source: &mut String, markdown: bool, read_only: bool, p: Palette, initial_cursor: usize, initial_scroll: f32) -> EditorResponse {
        if self.find_open {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Find").small().color(p.secondary));
                ui.add(egui::TextEdit::singleline(&mut self.find).id_salt((key,"find")).desired_width((ui.available_width()-70.0).max(80.0)).hint_text("Find in source\u{2026}"));
                if crate::icons::button(ui,crate::icons::Icon::Close,"Close find",false,true).clicked() {self.find_open=false;}
            });
            if !self.find.is_empty() {
                let count=source.to_lowercase().matches(&self.find.to_lowercase()).count();
                ui.label(RichText::new(format!("{count} matches · Enter in Find jumps to the next match")).small().color(p.muted));
            }
            ui.add_space(token::SPACE[1]);
        }
        let text_id=egui::Id::new(("source",key));
        let find=self.find.clone();
        let font=FontId::monospace(token::FONT_MONO);
        let mut layouter=|ui:&egui::Ui, text:&dyn egui::TextBuffer, _width:f32| {
            let mut job=highlight(text.as_str(),markdown,p,&find,font.clone());
            job.wrap.max_width=f32::INFINITY;
            ui.fonts_mut(|fonts|fonts.layout_job(job))
        };
        let mut cursor=initial_cursor;
        let mut changed=false;
        let mut focused=false;
        let mut area=egui::ScrollArea::both().id_salt(("editor-scroll",key)).auto_shrink([false,false]);
        let first_open=!ui.data(|data|data.get_temp::<bool>(text_id.with("opened")).unwrap_or(false));
        if first_open {
            area=area.vertical_scroll_offset(initial_scroll);
            ui.data_mut(|data|data.insert_temp(text_id.with("opened"),true));
        }
        let view_width=ui.available_width();
        let row_height=ui.fonts_mut(|fonts|fonts.row_height(&font));
        let view_rows=(ui.available_height()/row_height).ceil() as usize;
        let output=area.show(ui,|ui| {
            ui.horizontal_top(|ui| {
                let line_count=source.bytes().filter(|byte|*byte==b'\n').count()+1;
                // The numbers are painted after the text box, at the y of each galley row, so they
                // can never drift from the code lines.
                let digit_width=ui.fonts_mut(|fonts|fonts.glyph_width(&font,'0'));
                let gutter_width=(line_count.to_string().len().max(2) as f32*digit_width).ceil();
                let (gutter,_)=ui.allocate_exact_size(egui::vec2(gutter_width,1.0),egui::Sense::hover());
                // The text box is as wide as the viewport; longer lines scroll sideways.
                let desired_width=(view_width-gutter_width-ui.spacing().item_spacing.x-token::SPACE[1]).max(token::SPACE[5]*2.0);
                let edit=egui::TextEdit::multiline(source).id(text_id).font(FontId::monospace(token::FONT_MONO)).code_editor().frame(false).desired_width(desired_width).desired_rows(line_count.max(view_rows)).lock_focus(true).interactive(!read_only).layouter(&mut layouter);
                let mut response=edit.show(ui);
                paint_line_numbers(ui,&response.galley,response.galley_pos,gutter.right(),&font,p.muted);
                changed=response.response.changed();
                focused=response.response.has_focus();
                if let Some(range)=response.cursor_range {cursor=range.primary.index;}
                let mut target=self.jump_line.take().map(|line|source.split_inclusive('\n').take(line.saturating_sub(1)).map(|part|part.chars().count()).sum::<usize>()).or_else(||first_open.then_some(initial_cursor));
                if self.find_open&&!self.find.is_empty() && ui.input(|input|input.key_pressed(egui::Key::Enter)) {
                    let needle=self.find.to_lowercase();let length=self.find.chars().count();
                    let matches=source.char_indices().enumerate().filter(|(_, (offset,_))|source[*offset..].chars().take(length).collect::<String>().to_lowercase()==needle).map(|(character,_)|character).collect::<Vec<_>>();
                    target=matches.iter().copied().find(|position|*position>cursor).or_else(||matches.first().copied());
                }
                let selection=self.select.take().map(|(start,end)|{let total=source.chars().count();(start.min(total),end.min(total))});
                if let Some((start,end))=selection {
                    let range=egui::text::CCursorRange::two(egui::text::CCursor::new(start),egui::text::CCursor::new(end));
                    response.state.cursor.set_char_range(Some(range));response.state.store(ui.ctx(),text_id);response.response.request_focus();
                    let position=response.galley.pos_from_cursor(range.secondary).min.y;
                    ui.scroll_to_rect(egui::Rect::from_min_size(response.response.rect.min+egui::vec2(0.0,position),egui::vec2(10.0,22.0)),Some(egui::Align::Center));
                    cursor=end;focused=true;
                } else if let Some(target)=target {
                    let range=egui::text::CCursorRange::one(egui::text::CCursor::new(target.min(source.chars().count())));
                    response.state.cursor.set_char_range(Some(range));response.state.store(ui.ctx(),text_id);response.response.request_focus();
                    let position=response.galley.pos_from_cursor(range.primary).min.y;
                    ui.scroll_to_rect(egui::Rect::from_min_size(response.response.rect.min+egui::vec2(0.0,position),egui::vec2(10.0,22.0)),Some(egui::Align::Center));
                    cursor=target;
                }
            });
        });
        EditorResponse {changed,cursor,scroll:output.state.offset.y,focused}
    }
}

/// Paints one number per source line, right-aligned at `right`, centred on the galley row of that line.
fn paint_line_numbers(ui:&egui::Ui,galley:&egui::Galley,origin:egui::Pos2,right:f32,font:&FontId,color:Color32) {
    let clip=ui.clip_rect();
    let mut line=1;
    for row in &galley.rows {
        let y=origin.y+row.rect().center().y;
        if y>clip.top()-row.height()&&y<clip.bottom()+row.height() {
            ui.painter().text(egui::pos2(right,y),egui::Align2::RIGHT_CENTER,line.to_string(),font.clone(),color);
        }
        if row.ends_with_newline {line+=1;}
    }
}

fn append(job:&mut egui::text::LayoutJob,text:&str,color:Color32,font:&FontId,background:Color32) {
    job.append(text,0.0,TextFormat{font_id:font.clone(),color,background,..Default::default()});
}

fn highlight(source:&str,markdown:bool,p:Palette,find:&str,font:FontId)->egui::text::LayoutJob {
    let mut job=egui::text::LayoutJob::default();
    // Token colors are presentation only. The renderer owns Mermaid parsing.
    let keyword=p.accent;
    let string=if p.ink.r()>200 {Color32::from_rgb(142,186,150)}else{Color32::from_rgb(60,105,75)};
    let arrow=p.accent;
    let keywords=["flowchart","graph","sequenceDiagram","classDiagram","stateDiagram-v2","stateDiagram","erDiagram","gantt","pie","journey","mindmap","timeline","gitGraph","quadrantChart","requirementDiagram","subgraph","end","direction","classDef","class","style","click","participant","actor","loop","alt","else","opt","par","and","Note","note","activate","deactivate","title","section","dateFormat","accTitle","accDescr","TD","TB","BT","LR","RL"];
    let query=find.to_lowercase();
    for line in source.split_inclusive('\n') {
        if line.trim_start().starts_with("%%") || (markdown && line.trim_start().starts_with("<!--")) {append(&mut job,line,p.muted,&font,Color32::TRANSPARENT);continue;}
        if markdown&&(line.trim_start().starts_with('#')||line.trim_start().starts_with("```")) {append(&mut job,line,keyword,&font,Color32::TRANSPARENT);continue;}
        let mut offset=0;
        while offset<line.len() {
            let tail=&line[offset..];let first=tail.chars().next().unwrap();
            let (length,color)=if first=='"'||first=='\'' {
                let mut escaped=false;let mut end=first.len_utf8();for ch in tail[first.len_utf8()..].chars(){end+=ch.len_utf8();if ch==first&&!escaped{break;}escaped=ch=='\\'&&!escaped;}(end,string)
            }else if first.is_alphanumeric()||first=='_' {
                let length=tail.char_indices().find(|(_,ch)|!(ch.is_alphanumeric()||*ch=='_'||*ch=='-')).map(|(index,_)|index).unwrap_or(tail.len());
                let word=&tail[..length];(length,if keywords.contains(&word){keyword}else if word.chars().all(|ch|ch.is_numeric()){arrow}else{p.ink})
            }else{(first.len_utf8(),if "<>=|".contains(first){arrow}else{p.secondary})};
            let token=&tail[..length];let background=if !query.is_empty()&&token.to_lowercase().contains(&query){p.accent_soft}else{Color32::TRANSPARENT};
            append(&mut job,token,color,&font,background);offset+=length;
        }
    }
    job
}
