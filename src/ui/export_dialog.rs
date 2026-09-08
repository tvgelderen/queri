use super::{Services, controls::choice, field, task};
use crate::{
    export::{self, Format, Selection},
    model::ResultSet,
};
use gpui_kit::component::{
    ActiveTheme, Disableable, WindowExt,
    button::{Button, ButtonVariants},
    input::InputState,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
enum Scope {
    Loaded,
    Rows,
    Cells,
}
struct ExportDialog {
    services: Services,
    data: Arc<ResultSet>,
    rows: Option<Selection>,
    cells: Option<Selection>,
    scope: Scope,
    format: Format,
    null: Entity<InputState>,
    busy: bool,
    message: String,
}
pub fn open(
    services: Services,
    data: Arc<ResultSet>,
    rows: Option<Selection>,
    cells: Option<Selection>,
    window: &mut Window,
    cx: &mut App,
) {
    let form = cx.new(|cx| ExportDialog {
        services,
        data,
        rows,
        cells,
        scope: Scope::Loaded,
        format: Format::Csv,
        null: cx.new(|cx| InputState::new(window, cx).default_value(r"\N")),
        busy: false,
        message: String::new(),
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.title("Export data").child(form.clone())
    });
}
impl ExportDialog {
    fn selection(&self) -> Selection {
        match self.scope {
            Scope::Rows => self.rows.clone(),
            Scope::Cells => self.cells.clone(),
            Scope::Loaded => None,
        }
        .unwrap_or_else(|| Selection::all(&self.data))
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let data = self.data.clone();
        let selection = self.selection();
        let count = selection.rows.len();
        let format = self.format;
        let null = self.null.read(cx).value().to_string();
        let name = format!("results.{}", format.extension());
        let receiver = cx.prompt_for_new_path(std::path::Path::new("."), Some(&name));
        self.busy = true;
        self.message.clear();
        cx.spawn_in(window, async move |view,cx| {
            let result = receiver.await;
            let _ = view.update_in(cx, |this,window,cx| {
                match result {
                    Ok(Ok(Some(path))) => {
                        let destination = path.display().to_string();
                        task(&this.services,window,cx, async move {
                            tokio::task::spawn_blocking(move || export::atomic_write(&path, |writer|
                                export::serialize(&data,&selection,format,true,&null,writer))).await?
                        }, move |this,result,_,cx| {
                            this.busy = false;
                            this.message = match result {
                                Ok(()) => format!("Saved {count} rows to {destination}"),
                                Err(error) => format!("Could not export: {error:#}. Choose another destination and retry."),
                            };
                            cx.notify();
                        });
                    }
                    Ok(Err(error)) => { this.busy = false; this.message = format!("Could not open save dialog: {error}"); }
                    _ => this.busy = false,
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
}
impl Render for ExportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut scopes = vec![(Scope::Loaded, SharedString::from("Loaded rows"))];
        if self.rows.is_some() {
            scopes.push((Scope::Rows, "Selected rows".into()));
        }
        if self.cells.is_some() {
            scopes.push((Scope::Cells, "Selected cells".into()));
        }
        let label = match self.scope {
            Scope::Loaded => "Loaded rows",
            Scope::Rows => "Selected rows",
            Scope::Cells => "Selected cells",
        };
        v_flex().gap_4()
            .child(choice(Button::new("export-scope").label(label).disabled(self.busy).accessibility_label("Export scope"),self.scope,scopes,cx.listener(|this,value,_,cx|{ this.scope = *value; cx.notify(); })))
            .child(choice(Button::new("export-format").label(self.format.extension().to_uppercase()).disabled(self.busy).accessibility_label("Export format"),self.format,
                [(Format::Csv,"CSV".into()),(Format::Tsv,"TSV".into()),(Format::Json,"JSON · typed values".into())],
                cx.listener(|this,value,_,cx|{ this.format = *value; cx.notify(); })))
            .when(self.format != Format::Json, |view| view.child(field("NULL representation",&self.null))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Delimited text does not retain types. Use JSON to preserve exact values.")))
            .child(div().text_sm().child(format!("{} rows · {} columns · loaded data only{}",self.selection().rows.len(),self.selection().columns.len(),
                if self.data.truncated { " · result was truncated" } else { "" })))
            .child(div().id("export-status").role(Role::Status).text_sm().children((!self.message.is_empty()).then(||self.message.clone())))
            .child(Button::new("save-export").primary().label(if self.busy { "Saving…" } else { "Choose destination…" }).disabled(self.busy)
                .on_click(cx.listener(|this,_,window,cx| this.save(window,cx))))
    }
}
