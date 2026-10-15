use chrono::{DateTime, Local, Utc};
use iced::Element;
use iced::widget::{
    button, checkbox, column, container, image, mouse_area,
    operation::focus,
    pick_list, row, scrollable,
    space::{horizontal, vertical},
    text, text_input, tooltip,
};
use iced::{Color, Length, Task};
use indexmap::{IndexMap, map::MutableEntryKey};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::{fs::File, path::Path};
use typst::foundations::{Dict, Value};
use typst_as_lib::{TypstEngine, TypstTemplateMainFile};
use typst_iced_editor::{Action, Content, code_editor};
use typst_layout::PagedDocument;
use zip::ZipWriter;
use zip::read::ZipArchive;
use zip::write::FileOptions;

macro_rules! current_user {
    () => {
        whoami::realname().ok().or_else(|| whoami::username().ok())
    };
}

static FONT: &[u8] = include_bytes!("./DejaVuSans.ttf");

fn main() {
    iced::application(State::new, State::update, State::view)
        .run()
        .unwrap()
}

#[derive(Default)]
struct State {
    filepath: Option<PathBuf>,
    project: Project,
    engine: Option<TypstEngine<TypstTemplateMainFile>>,
    document: Option<PagedDocument>,
    alternative: Option<String>,
    edit_variable: Option<(usize, bool, String)>,
    page: Option<image::Handle>,
    curr_page: usize,
    query: Option<Query>,
}

impl State {
    fn new() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.as_slice() {
            [] => State::default(),
            [path] => {
                let project = Project::from_file(path.as_ref()).unwrap();
                let engine = Some(project.typst_engine());
                State {
                    project,
                    engine,
                    ..Default::default()
                }
            }
            [..] => panic!("invalid inputs"),
        }
    }

    fn variables(&self) -> &IndexMap<String, Variable> {
        let alt = if let Some(alt) = &self.alternative {
            &self.project.alternatives[alt]
        } else {
            &self.project.variables
        };
        &alt.variables
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct ExportOptions {
    format: ExportFormat,
    // document: String,
    filename: String,
    pages: String,
    export_all: bool,
}

enum TemplatePart<'a> {
    Var(&'a str),
    Lit(&'a str),
    Page(&'a str, usize),
}

/// Template for the names, variables inside {}, page numbers are variables that end in one or more #
/// multiple # are zero padded
fn parse_templ(name: &str) -> Vec<TemplatePart<'_>> {
    let mut parts = vec![];
    let mut pointer = 0usize;
    let mut variable = false;
    for (i, c) in name.char_indices() {
        match c {
            '{' if !variable => {
                variable = true;
                if pointer != i {
                    parts.push(TemplatePart::Lit(&name[pointer..i]));
                }
                pointer = i + 1;
            }
            '}' if variable => {
                variable = false;
                if pointer != i {
                    let var = &name[pointer..i];
                    if var.chars().rev().next() == Some('#') {
                        let pre = var.trim_end_matches('#');
                        parts.push(TemplatePart::Page(
                            &name[pointer..(pointer + pre.len())],
                            var.len() - pre.len(),
                        ));
                    } else {
                        parts.push(TemplatePart::Var(var));
                    }
                }
                pointer = i + 1;
            }
            _ => (),
        }
    }
    parts
}

fn render_templ(
    parts: &[TemplatePart],
    variables: &IndexMap<String, Variable>,
    page: Option<usize>,
) -> String {
    let mut result = String::new();
    for p in parts {
        match p {
            TemplatePart::Lit(l) => result.push_str(l),
            TemplatePart::Var(v) => result.push_str(
                &variables
                    .get(*v)
                    .map(|x| x.value.clone())
                    // maybe give error message if variable not found
                    .unwrap_or(format!("{{{v}}}")),
            ),
            TemplatePart::Page(v, p) => {
                if let Some(num) = page {
                    result.push_str(v);
                    result.push_str(&format!("{:0width$}", num, width = p))
                }
            }
        }
    }
    result
}

fn numrng(rng: &str) -> Vec<usize> {
    rng.split(",")
        .map(|v| match v.split_once("-") {
            Some((a, b)) => {
                let a = a.parse::<usize>().unwrap();
                let b = b.parse::<usize>().unwrap();
                a..=b
            }
            None => {
                let a = v.parse::<usize>().unwrap();
                a..=a
            }
        })
        .flatten()
        .collect()
}

#[derive(Default, Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
enum ExportFormat {
    #[default]
    Pdf,
    Svg,
    Png,
}

impl ExportFormat {
    fn all() -> &'static [ExportFormat] {
        &[ExportFormat::Pdf, ExportFormat::Svg, ExportFormat::Png]
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self {
            Self::Pdf => write!(f, "PDF"),
            Self::Svg => write!(f, "SVG"),
            Self::Png => write!(f, "PNG"),
        }
    }
}

enum Query {
    NewAlternative(String, Option<String>),
    RenameAlternative(usize, String),
    EditTemplate(Content),
    EditDescription(Content),
    ExportFile(ExportOptions),
}

impl Query {
    fn view<'a>(&'a self, alts: &'a [String]) -> Element<'a, Message> {
        let (title, q): (&'static str, Element<_>) = match self {
            Query::NewAlternative(name, from) => (
                "Add a new Alternative",
                row![
                    text_input("Name", &name).on_input(Message::QueryStringEdit),
                    pick_list(alts, from.as_ref(), Message::QueryDropDownChanged),
                ]
                .width(Length::FillPortion(1))
                .into(),
            ),
            Query::RenameAlternative(_, name) => (
                "Rename Alternative",
                row![
                    text_input("Name", &name)
                        .on_input(Message::QueryStringEdit)
                        .on_submit(Message::SubmitQuery)
                ]
                .width(Length::FillPortion(1))
                .into(),
            ),
            Query::EditTemplate(content) => (
                "Edit Typst Template",
                code_editor(&content)
                    .on_action(Message::QueryEditorAction)
                    .height(Length::Fill)
                    .width(Length::FillPortion(4))
                    .into(),
            ),
            Query::EditDescription(content) => (
                "Edit Description",
                code_editor(&content)
                    .on_action(Message::QueryEditorAction)
                    .height(Length::Fill)
                    .width(Length::FillPortion(1))
                    .into(),
            ),
            Query::ExportFile(opt) => (
                "Export File",
                column![
                    row![
                        text("Export Format:").width(Length::FillPortion(1)),
                        pick_list(
                            ExportFormat::all(),
                            Some(&opt.format),
                            Message::ExportFormatChanged
                        )
                        .width(Length::FillPortion(2)),
                    ]
                    .spacing(10)
                    .width(Length::FillPortion(1)),
                    row![
                        text("File Name:").width(Length::FillPortion(1)),
                        text_input("File Name", &opt.filename)
                            .on_input(Message::QueryStringEdit)
                            .width(Length::FillPortion(2)),
                    ]
                    .spacing(10)
                    .width(Length::FillPortion(1)),
                    row![
                        text("Pages:").width(Length::FillPortion(1)),
                        text_input("All Pages", &opt.pages)
                            .on_input(Message::QueryString2Edit)
                            .width(Length::FillPortion(2)),
                    ]
                    .spacing(10)
                    .width(Length::FillPortion(1)),
                    row![
                        checkbox(opt.export_all)
                            .label("Export All Alternatives")
                            .on_toggle(Message::QueryCheckboxToggle)
                            .width(Length::FillPortion(2)),
                    ]
                    .spacing(10)
                    .width(Length::FillPortion(1))
                ]
                .spacing(10)
                .into(),
            ),
        };
        column![
            text(title),
            q,
            row![
                button("Submit").on_press(Message::SubmitQuery),
                horizontal(),
                button("Cancel").on_press(Message::CancelQuery)
            ]
            .spacing(25)
        ]
        .spacing(10)
        .padding(30)
        .into()
    }
}

#[derive(Clone)]
enum Message {
    OpenProject,
    SaveProject,
    RefreshPage,
    SubmitQuery,
    CancelQuery,
    EditTemplate,
    QueryEditorAction(Action),
    QueryStringEdit(String),
    QueryString2Edit(String),
    QueryDropDownChanged(String),
    QueryCheckboxToggle(bool),
    ExportFormatChanged(ExportFormat),
    ProjectNameChange(String),
    ProjectDescriptionChange(String),
    MultiLineDescription,
    ShowBaseCase,
    NewAlternative,
    AlternativeSelect(String),
    StartEdit(usize, bool),
    EditVariableField(String),
    SubmitVariableEdit,
    NewVariable,
    StartExport,
    RenameResource(usize),
    DeleteResource(usize),
    DownloadResource(usize),
    UploadResource,
    UploadFile(Option<PathBuf>),
    NextPage,
    PreviousPage,
    PageChange(String),
}

struct TypDoc;

impl State {
    fn name() -> &'static str {
        "TypDoc"
    }

    fn boot(&self) -> (State, Task<Message>) {
        (State::default(), Task::none())
    }

    fn apply_variable_change(&mut self) -> bool {
        let alt = if let Some(alt) = &self.alternative {
            &mut self.project.alternatives[alt]
        } else {
            &mut self.project.variables
        };
        match self.edit_variable.take() {
            None => true,
            Some((ind, true, val)) => {
                let val = val.trim();
                if alt.variables.replace_index(ind, val.to_string()).is_ok() {
                    let v = &mut alt.variables[val];
                    if v.name != val {
                        v.name = val.to_string();
                        v.last_modified = Utc::now();
                        v.modified_by = current_user!();
                    }
                    true
                } else {
                    self.edit_variable = Some((ind, true, val.to_string()));
                    false
                }
            }
            Some((ind, false, val)) => {
                if let Some((_, v)) = alt.variables.get_index_mut(ind) {
                    if v.value != val {
                        v.value = val;
                        v.last_modified = Utc::now();
                        v.modified_by = current_user!();
                    }
                    true
                } else {
                    true
                }
            }
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenProject => {
                self.project = Project::from_file("test.typdoc".as_ref()).unwrap();
                self.query = None;
                self.edit_variable = None;
                return Task::done(Message::RefreshPage);
            }
            Message::SaveProject => {
                self.project.save_file("test.typdoc".as_ref()).unwrap();
            }
            Message::UploadResource => {
                return Task::perform(upload_file_dialog(), Message::UploadFile);
            }
            Message::UploadFile(None) => (),
            Message::UploadFile(Some(filename)) => {
                self.project.add_resource(filename);
                self.engine.replace(self.project.typst_engine());
            }
            Message::RefreshPage => {
                let alt = if let Some(alt) = &self.alternative {
                    &self.project.alternatives[alt]
                } else {
                    &self.project.variables
                };
                let vars: Dict = alt
                    .variables
                    .iter()
                    .map(|(k, v)| (k.as_str().into(), Value::Str(v.value.clone().into())))
                    .collect();

                let eng = self
                    .engine
                    .get_or_insert_with(|| self.project.typst_engine());
                self.document = match eng.compile_with_input(vars).output {
                    Ok(s) => Some(s),
                    Err(e) => {
                        eprintln!("Error compiling: {e:?}");
                        None
                    }
                };
                if let Some(doc) = &self.document {
                    let opt = typst_render::RenderOptions::default();
                    let page = match doc.pages().get(self.curr_page) {
                        Some(p) => &p,
                        None => {
                            self.curr_page = 0;
                            &doc.pages()[0]
                        }
                    };
                    let pixmap = typst_render::render(page, &opt);
                    self.page = Some(image::Handle::from_rgba(
                        pixmap.width(),
                        pixmap.height(),
                        pixmap.data().to_vec(),
                    ));
                }
            }
            Message::PageChange(new) => {
                if let Some(d) = &self.document {
                    if let Ok(p) = new.parse::<usize>() {
                        let opt = typst_render::RenderOptions::default();
                        match d.pages().get(p - 1) {
                            Some(page) => {
                                self.curr_page = p - 1;
                                let pixmap = typst_render::render(page, &opt);
                                self.page = Some(image::Handle::from_rgba(
                                    pixmap.width(),
                                    pixmap.height(),
                                    pixmap.data().to_vec(),
                                ));
                            }
                            None => (),
                        }
                        return Task::none();
                    }
                }
            }
            Message::NextPage => {
                if let Some(d) = &self.document {
                    let opt = typst_render::RenderOptions::default();
                    match d.pages().get(self.curr_page + 1) {
                        Some(page) => {
                            self.curr_page += 1;
                            let pixmap = typst_render::render(page, &opt);
                            self.page = Some(image::Handle::from_rgba(
                                pixmap.width(),
                                pixmap.height(),
                                pixmap.data().to_vec(),
                            ));
                        }
                        None => return Task::none(),
                    }
                }
            }
            Message::PreviousPage => {
                if let Some(d) = &self.document {
                    let opt = typst_render::RenderOptions::default();
                    match d.pages().get(self.curr_page - 1) {
                        Some(page) => {
                            self.curr_page -= 1;
                            let pixmap = typst_render::render(page, &opt);
                            self.page = Some(image::Handle::from_rgba(
                                pixmap.width(),
                                pixmap.height(),
                                pixmap.data().to_vec(),
                            ));
                        }
                        None => return Task::none(),
                    }
                }
            }
            Message::SubmitQuery => {
                let q = match self.query.take() {
                    Some(q) => q,
                    // invalid state
                    None => return Task::none(),
                };
                match q {
                    Query::NewAlternative(name, from) => {
                        self.project.add_alternative(name.clone(), from);
                        self.alternative = Some(name)
                    }
                    Query::RenameAlternative(ind, name) => {
                        self.project.rename_alternative(ind, name.clone());
                        self.alternative = Some(name)
                    }
                    Query::EditTemplate(content) => {
                        self.project.template = content.text();
                        self.engine = Some(self.project.typst_engine());
                    }
                    Query::EditDescription(content) => {
                        self.project.metadata.description = content.text();
                    }
                    Query::ExportFile(opt) => {
                        if opt.export_all {
                            //
                        } else {
                            if let Some(d) = &self.document {
                                let mut templ = parse_templ(&opt.filename);
                                match opt.format {
                                    ExportFormat::Pdf => {
                                        let options = Default::default();
                                        let pdf = typst_pdf::pdf(d, &options)
                                            .expect("Could not generate pdf.");
                                        let filename = render_templ(&templ, self.variables(), None);
                                        std::fs::write(
                                            Path::new(&filename).with_extension("pdf"),
                                            pdf,
                                        )
                                        .expect("Could not write pdf.");
                                    }
                                    ExportFormat::Png => {
                                        if d.pages().len() > 1
                                            && !templ
                                                .iter()
                                                .any(|t| matches!(t, TemplatePart::Page(..)))
                                        {
                                            // if no page part in template, add -###
                                            templ.push(TemplatePart::Page("-", 3));
                                        }
                                        for (i, page) in d.pages().iter().enumerate() {
                                            let opt = typst_render::RenderOptions::default();
                                            let pixmap = typst_render::render(page, &opt);
                                            let filename =
                                                render_templ(&templ, self.variables(), Some(i));
                                            pixmap
                                                .save_png(
                                                    Path::new(&filename).with_extension("png"),
                                                )
                                                .unwrap();
                                        }
                                    }
                                    _ => (),
                                }
                            }
                        }
                        // update this for next time; doing it this
                        // way so that cancelled Query doesn't
                        // overwrite the export options in the project
                        self.project.export_options = Some(opt);
                    }
                };
                return Task::done(Message::RefreshPage);
            }
            Message::CancelQuery => {
                self.query = None;
            }
            Message::QueryStringEdit(val) => {
                match self.query.take() {
                    Some(Query::NewAlternative(_, from)) => {
                        self.query = Some(Query::NewAlternative(val, from));
                    }
                    Some(Query::RenameAlternative(ind, _)) => {
                        self.query = Some(Query::RenameAlternative(ind, val));
                    }
                    Some(Query::ExportFile(mut opt)) => {
                        opt.filename = val;
                        self.query = Some(Query::ExportFile(opt));
                    }
                    q => {
                        self.query = q;
                    }
                };
            }
            Message::ExportFormatChanged(val) => {
                match self.query.take() {
                    Some(Query::ExportFile(mut opt)) => {
                        opt.format = val;
                        self.query = Some(Query::ExportFile(opt));
                    }
                    q => {
                        self.query = q;
                    }
                };
            }
            Message::QueryString2Edit(val) => {
                match self.query.take() {
                    Some(Query::ExportFile(mut opt)) => {
                        opt.pages = val;
                        self.query = Some(Query::ExportFile(opt));
                    }
                    q => {
                        self.query = q;
                    }
                };
            }
            Message::QueryCheckboxToggle(val) => {
                match self.query.take() {
                    Some(Query::ExportFile(mut opt)) => {
                        opt.export_all = val;
                        self.query = Some(Query::ExportFile(opt));
                    }
                    q => {
                        self.query = q;
                    }
                };
            }
            Message::QueryEditorAction(act) => {
                match &mut self.query {
                    Some(Query::EditTemplate(content)) => {
                        content.perform(act);
                    }
                    Some(Query::EditDescription(content)) => {
                        content.perform(act);
                    }
                    _ => (),
                };
            }
            Message::ProjectNameChange(name) => {
                self.project.metadata.name = name;
            }
            Message::ProjectDescriptionChange(desc) => {
                self.project.metadata.description = desc;
            }
            Message::ShowBaseCase => {
                self.alternative = None;
                return Task::done(Message::RefreshPage);
            }
            Message::NewAlternative => {
                self.query = Some(Query::NewAlternative(String::new(), None));
            }
            Message::StartExport => {
                let opt = self
                    .project
                    .export_options
                    .clone()
                    .unwrap_or_else(|| ExportOptions {
                        filename: "document{-##}".into(),
                        ..Default::default()
                    });
                self.query = Some(Query::ExportFile(opt));
            }
            Message::MultiLineDescription => {
                self.query = Some(Query::EditDescription(Content::with_text(
                    &self.project.metadata.description,
                )));
            }
            Message::EditTemplate => {
                self.query = Some(Query::EditTemplate(Content::with_text(
                    &self.project.template,
                )));
            }
            Message::AlternativeSelect(alt) => {
                self.alternative = Some(alt);
                return Task::done(Message::RefreshPage);
            }
            Message::EditVariableField(fd) => {
                if let Some((_, _, val)) = &mut self.edit_variable {
                    *val = fd;
                }
            }
            Message::SubmitVariableEdit => {
                if self.apply_variable_change() {
                    return Task::done(Message::RefreshPage);
                }
            }
            Message::NewVariable => {
                self.apply_variable_change();
                if self.edit_variable.is_none() {
                    if let Some(alt) = &self.alternative {
                        self.edit_variable = Some((
                            self.project.alternatives[alt].variables.len(),
                            true,
                            String::new(),
                        ));
                        self.project.alternatives[alt]
                            .variables
                            .insert(String::new(), Variable::default());
                    } else {
                        self.edit_variable =
                            Some((self.project.variables.variables.len(), true, String::new()));
                        self.project
                            .variables
                            .variables
                            .insert(String::new(), Variable::default());
                    }
                    return focus("currentEdit");
                }
            }
            Message::StartEdit(ind, key) => {
                self.apply_variable_change();
                if self.edit_variable.is_none() {
                    let alt = if let Some(alt) = &self.alternative {
                        &self.project.alternatives[alt]
                    } else {
                        &self.project.variables
                    };
                    if let Some((k, v)) = alt.variables.get_index(ind) {
                        let val = if key {
                            k.to_string()
                        } else {
                            v.value.to_string()
                        };
                        self.edit_variable = Some((ind, key, val));
                        return focus("currentEdit");
                    }
                }
            }
            _ => (),
        }
        Task::none()
    }

    fn view<'a>(&'a self) -> Element<'a, Message> {
        row![self.left_pane(), self.right_pane()]
            .spacing(20)
            .padding(20)
            .width(Length::Fill)
            .into()
    }

    fn left_pane<'a>(&'a self) -> Element<'a, Message> {
        let resources = self
            .project
            .resources
            .iter()
            .enumerate()
            .map(|(i, (k, v))| {
                row![
                    mouse_area(
                        container(text(k))
                            .padding(5)
                            .style(container::bordered_box)
                            .width(Length::FillPortion(2))
                    )
                    .on_press(Message::RenameResource(i)),
                    tooltip(
                        container(text(v.relative_time()))
                            .padding(5)
                            .style(container::bordered_box)
                            .width(Length::FillPortion(1)),
                        container(text(v.modified_by())).style(container::rounded_box),
                        tooltip::Position::Top,
                    ),
                    horizontal().width(25),
                    button("v").on_press(Message::DownloadResource(i)),
                    button("x").on_press(Message::DeleteResource(i)),
                ]
                .spacing(10)
                .into()
            })
            .collect();

        column![
            row![
                button("Open Project").on_press(Message::OpenProject),
                horizontal(),
                button("Save Project").on_press(Message::SaveProject),
            ],
            row![
                text("Project Name:").width(Length::FillPortion(1)),
                text_input("Name", &self.project.metadata.name)
                    .on_input(Message::ProjectNameChange)
                    .width(Length::FillPortion(3))
            ]
            .width(Length::Fill),
            row![
                text("Description:").width(Length::FillPortion(1)),
                row![
                    text_input(
                        "Description",
                        &self
                            .project
                            .metadata
                            .description
                            .split("\n")
                            .next()
                            .unwrap_or("")
                    )
                    .width(Length::Fill)
                    .on_input_maybe(
                        (!self.project.metadata.description.contains("\n"))
                            .then_some(Message::ProjectDescriptionChange)
                    ),
                    tooltip(
                        button("…").on_press_maybe(
                            self.query.is_none().then(|| Message::MultiLineDescription)
                        ),
                        container(text(&self.project.metadata.description).width(300))
                            .style(container::rounded_box)
                            .padding(10),
                        tooltip::Position::Top,
                    )
                ]
                .width(Length::FillPortion(3)),
            ]
            .width(Length::Fill),
            row![
                text("Alternative:").width(Length::FillPortion(1)),
                row![
                    pick_list(
                        self.project.alt_list.as_ref(),
                        self.alternative.as_ref(),
                        Message::AlternativeSelect
                    )
                    .placeholder("Base Case")
                    .width(Length::Fill),
                    button("+")
                        .on_press_maybe(self.query.is_none().then(|| Message::NewAlternative))
                ]
                .width(Length::FillPortion(3)),
            ]
            .width(Length::Fill),
            text("Variables"),
            container(
                scrollable(match &self.alternative {
                    Some(alt) => self.project.alternatives[alt].view(&self.edit_variable),
                    None => self.project.variables.view(&self.edit_variable),
                })
                .height(Length::FillPortion(1))
            )
            .style(container::bordered_box)
            .padding(5),
            row![
                text("Resources"),
                horizontal().width(Length::Fill),
                button("Upload")
                    .on_press_maybe(self.query.is_none().then(|| Message::UploadResource)),
            ],
            container(
                scrollable(
                    iced::widget::Column::from_vec(resources)
                        .spacing(10)
                        .width(Length::Fill)
                )
                .height(Length::FillPortion(1))
            )
            .style(container::bordered_box)
            .padding(5),
            row![
                button("Show Base Case").on_press(Message::ShowBaseCase),
                horizontal().width(Length::Fill),
                button("Edit Template")
                    .on_press_maybe(self.query.is_none().then(|| Message::EditTemplate)),
            ]
        ]
        .spacing(10)
        .width(Length::FillPortion(2))
        .into()
    }

    fn right_pane<'a>(&'a self) -> Element<'a, Message> {
        if let Some(query) = &self.query {
            query.view(&self.project.alt_list)
        } else {
            column![
                row![
                    button("Refresh").on_press(Message::RefreshPage),
                    horizontal().width(Length::Fill),
                    button("Export").on_press(Message::StartExport)
                ],
                row![
                    button("<").on_press(Message::PreviousPage),
                    horizontal().width(Length::FillPortion(1)),
                    text_input::TextInput::new("P", &format!("{}", self.curr_page + 1))
                        .on_input(Message::PageChange)
                        .width(50),
                    text("/"),
                    text(self.document.as_ref().map(|d| d.pages().len()).unwrap_or(1)),
                    horizontal().width(Length::FillPortion(1)),
                    button(">").on_press(Message::NextPage),
                ]
                .height(25),
                self.page()
            ]
            .spacing(25)
            .width(Length::FillPortion(2))
            .into()
        }
    }

    fn page<'a>(&'a self) -> Element<'a, Message> {
        if let Some(p) = &self.page {
            image(p).into()
        } else {
            text("Compile to see preview").into()
        }
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct Project {
    metadata: Metadata,
    #[serde(skip)]
    template: String,
    template_info: TemplateInfo,
    variables: Alternative,
    resources: IndexMap<String, Resource>,
    alternatives: IndexMap<String, Alternative>,
    export_options: Option<ExportOptions>,
    // redundancy for the pick_list
    alt_list: Vec<String>,
}

impl Project {
    fn add_alternative(&mut self, name: String, from: Option<String>) {
        if self.alternatives.contains_key(&name) {
            return;
        }

        match from {
            Some(f) => {
                let mut old = self.alternatives[&f].clone();
                old.name = name.clone();
                self.alternatives.insert(name.clone(), old);
            }
            None => {
                self.alternatives
                    .insert(name.clone(), Alternative::new(name.clone()));
            }
        }
        self.alt_list.push(name);
    }

    fn rename_alternative(&mut self, index: usize, name: String) {
        if self.alternatives.contains_key(&name) {
            return;
        }

        match self.alternatives.get_index_entry(index) {
            Some(mut f) => {
                *f.key_mut() = name.clone();
                self.alt_list[index] = name;
            }
            None => {
                return;
            }
        }
    }

    fn add_resource(&mut self, filename: PathBuf) {
        let fname = filename.file_name().unwrap().to_string_lossy().to_string();
        let mut file = std::fs::File::open(&filename).unwrap();
        let mut data: Vec<u8> = Vec::new();
        file.read_to_end(&mut data).unwrap();
        self.resources.insert(
            fname,
            Resource {
                path: filename,
                contents: data,
                last_modified: Utc::now(),
                uploaded_by: current_user!(),
            },
        );
    }

    fn save_file(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let file = File::create(path)?;
        let mut zip = ZipWriter::new(file);

        zip.start_file("config.json", FileOptions::<()>::default())?;
        let json_config = serde_json::to_string(&self)?;
        zip.write_all(json_config.as_bytes())?;

        zip.start_file("template.typ", FileOptions::<()>::default())?;
        zip.write_all(self.template.as_bytes())?;

        for (name, res) in &self.resources {
            zip.start_file(name, FileOptions::<()>::default())?;
            zip.write_all(&res.contents)?;
        }

        zip.finish()?;
        Ok(())
    }

    fn from_file(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let mut archive = ZipArchive::new(file)?;

        let mut project: Project = {
            let mut file = archive.by_name("config.json")?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            let contents = String::from_utf8(buffer)?;
            serde_json::from_str(&contents)?
        };
        project.template = {
            let mut file = archive.by_name("template.typ")?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            String::from_utf8(buffer)?
        };

        for (name, res) in &mut project.resources {
            let mut file = archive.by_name(name)?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            res.contents = buffer.clone();
        }

        Ok(project)
    }

    fn typst_engine(&self) -> TypstEngine<TypstTemplateMainFile> {
        let res: Vec<(&str, &[u8])> = self
            .resources
            .iter()
            .map(|(k, v)| (k.as_str(), v.contents.as_slice()))
            .collect();
        TypstEngine::builder()
            .with_static_file_resolver(res)
            .main_file(self.template.clone())
            // might as well let users upload fonts; keep one as backup
            .fonts([FONT])
            .build()
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct Resource {
    path: PathBuf,
    #[serde(skip)]
    contents: Vec<u8>,
    last_modified: DateTime<Utc>,
    uploaded_by: Option<String>,
}

impl Resource {
    fn relative_time(&self) -> String {
        let duration = Utc::now().signed_duration_since(&self.last_modified);
        if duration.num_seconds() < 60 {
            "<1 m".to_string()
        } else if duration.num_minutes() < 60 {
            format!("{} m", duration.num_minutes())
        } else if duration.num_hours() < 24 {
            format!("{} h", duration.num_hours())
        } else {
            format!("{} d", duration.num_days())
        }
    }

    fn modified_by(&self) -> String {
        format!(
            "By {} at {}",
            self.uploaded_by.as_deref().unwrap_or("<unknown>"),
            self.last_modified
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
        )
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct TemplateInfo {
    typst_version: String,
    /// When was the content last modified
    last_modified: DateTime<Utc>,
    /// The user that last modified the template
    modified_by: String,
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct Metadata {
    name: String,
    description: String,
    last_modified: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Alternative {
    name: String,
    variables: IndexMap<String, Variable>,
}

impl Default for Alternative {
    fn default() -> Self {
        Self {
            name: "Base".to_string(),
            variables: IndexMap::new(),
        }
    }
}

impl Alternative {
    fn view<'a>(&'a self, edit: &Option<(usize, bool, String)>) -> Element<'a, Message> {
        let mut vars: Vec<Element<_>> = if let Some((ind, key, val)) = edit {
            self.variables
                .iter()
                .enumerate()
                .map(|(i, (k, v))| {
                    if &i == ind {
                        if *key {
                            row![
                                text_input("Name", val)
                                    .id("currentEdit")
                                    .on_input(Message::EditVariableField)
                                    .on_submit(Message::SubmitVariableEdit)
                                    .width(Length::FillPortion(1)),
                                mouse_area(
                                    container(text(&v.value))
                                        .padding(5)
                                        .style(container::bordered_box)
                                        .width(Length::FillPortion(3))
                                )
                                .on_press(Message::StartEdit(i, false)),
                                tooltip(
                                    container(text(v.relative_time()))
                                        .padding(5)
                                        .style(container::bordered_box)
                                        .width(Length::FillPortion(1)),
                                    container(text(v.modified_by())).style(container::rounded_box),
                                    tooltip::Position::Top,
                                ),
                            ]
                            .spacing(10)
                        } else {
                            row![
                                mouse_area(
                                    container(text(k))
                                        .padding(5)
                                        .style(container::bordered_box)
                                        .width(Length::FillPortion(1))
                                )
                                .on_press(Message::StartEdit(i, true)),
                                text_input("Value", val)
                                    .id("currentEdit")
                                    .on_input(Message::EditVariableField)
                                    .on_submit(Message::SubmitVariableEdit)
                                    .width(Length::FillPortion(3)),
                                tooltip(
                                    container(text(v.relative_time()))
                                        .padding(5)
                                        .style(container::bordered_box)
                                        .width(Length::FillPortion(1)),
                                    container(text(v.modified_by())).style(container::rounded_box),
                                    tooltip::Position::Top,
                                ),
                            ]
                            .spacing(10)
                        }
                    } else {
                        row![
                            mouse_area(
                                container(text(k))
                                    .padding(5)
                                    .style(container::bordered_box)
                                    .width(Length::FillPortion(1))
                            )
                            .on_press(Message::StartEdit(i, true)),
                            mouse_area(
                                container(text(&v.value))
                                    .padding(5)
                                    .style(container::bordered_box)
                                    .width(Length::FillPortion(3))
                            )
                            .on_press(Message::StartEdit(i, false)),
                            tooltip(
                                container(text(v.relative_time()))
                                    .padding(5)
                                    .style(container::bordered_box)
                                    .width(Length::FillPortion(1)),
                                container(text(v.modified_by())).style(container::rounded_box),
                                tooltip::Position::Top,
                            ),
                        ]
                        .spacing(10)
                    }
                    .into()
                })
                .collect()
        } else {
            self.variables
                .iter()
                .enumerate()
                .map(|(i, (k, v))| {
                    row![
                        mouse_area(
                            container(text(k))
                                .padding(5)
                                .style(container::bordered_box)
                                .width(Length::FillPortion(1))
                        )
                        .on_press(Message::StartEdit(i, true)),
                        mouse_area(
                            container(text(&v.value))
                                .padding(5)
                                .style(container::bordered_box)
                                .width(Length::FillPortion(3))
                        )
                        .on_press(Message::StartEdit(i, false)),
                        tooltip(
                            container(text(v.relative_time()))
                                .padding(5)
                                .style(container::bordered_box)
                                .width(Length::FillPortion(1)),
                            container(text(v.modified_by())).style(container::rounded_box),
                            tooltip::Position::Top,
                        ),
                    ]
                    .spacing(10)
                    .into()
                })
                .collect()
        };
        if edit.is_none() {
            vars.push(
                row![
                    mouse_area(
                        container(text("..New..").color(Color::WHITE.scale_alpha(0.4)))
                            .padding(5)
                            .style(container::bordered_box)
                            .width(Length::FillPortion(1))
                    )
                    .on_press(Message::NewVariable),
                    container(horizontal())
                        .padding(5)
                        .width(Length::FillPortion(3)),
                    container(horizontal())
                        .padding(5)
                        .width(Length::FillPortion(1)),
                ]
                .spacing(10)
                .into(),
            );
        }
        iced::widget::Column::from_vec(vars)
            .width(Length::Fill)
            .spacing(5)
            .into()
    }
}

impl Alternative {
    fn new(name: String) -> Alternative {
        Alternative {
            name,
            variables: IndexMap::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Variable {
    name: String,
    value: String,
    last_modified: DateTime<Utc>,
    modified_by: Option<String>,
}

impl Default for Variable {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: String::new(),
            last_modified: Utc::now(),
            modified_by: current_user!(),
        }
    }
}

impl Variable {
    fn relative_time(&self) -> String {
        let duration = Utc::now().signed_duration_since(&self.last_modified);
        if duration.num_seconds() < 60 {
            "<1 m".to_string()
        } else if duration.num_minutes() < 60 {
            format!("{} m", duration.num_minutes())
        } else if duration.num_hours() < 24 {
            format!("{} h", duration.num_hours())
        } else {
            format!("{} d", duration.num_days())
        }
    }

    fn modified_by(&self) -> String {
        format!(
            "By {} at {}",
            self.modified_by.as_deref().unwrap_or("<unknown>"),
            self.last_modified
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
        )
    }
}

async fn upload_file_dialog() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Upload a file")
        .pick_file()
        .await
        .map(|p| p.into())
}

async fn save_file_dialog() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .save_file()
        .await
        .map(|p| p.into())
}
