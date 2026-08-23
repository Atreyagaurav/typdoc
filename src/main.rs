use chrono::{DateTime, Utc};
use iced::Element;
use iced::widget::{
    button, column, container, image, mouse_area,
    operation::focus,
    pick_list, row, scrollable,
    space::{horizontal, vertical},
    text, text_input,
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
}

#[derive(Clone)]
struct ExportOptions {
    format: String,
    document: String,
    filename: String,
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
                .into(),
            ),
            Query::RenameAlternative(_, name) => (
                "Rename Alternative",
                row![
                    text_input("Name", &name)
                        .on_input(Message::QueryStringEdit)
                        .on_submit(Message::SubmitQuery)
                ]
                .into(),
            ),
            Query::EditTemplate(content) => (
                "Edit Typst Template",
                code_editor(&content)
                    .on_action(Message::QueryEditorAction)
                    .height(Length::Fill)
                    .into(),
            ),
            Query::EditDescription(content) => (
                "Edit Description",
                code_editor(&content)
                    .on_action(Message::QueryEditorAction)
                    .height(Length::Fill)
                    .into(),
            ),
            Query::ExportFile(_) => ("Export File", text("todo").into()),
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
    QueryDropDownChanged(String),
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
                    v.name = val.to_string();
                    v.last_modified = Utc::now();
                    true
                } else {
                    self.edit_variable = Some((ind, true, val.to_string()));
                    false
                }
            }
            Some((ind, false, val)) => {
                if let Some((_, v)) = alt.variables.get_index_mut(ind) {
                    v.value = val;
                    v.last_modified = Utc::now();
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

                let eng = self.project.typst_engine();
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
                    pixmap.save_png("test.png").unwrap();
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
                        let total = d.pages().len();
                        if p <= total && p > 0 {
                            self.curr_page = p - 1;
                            return Task::done(Message::RefreshPage);
                        }
                    }
                }
            }
            Message::NextPage => {
                if let Some(d) = &self.document {
                    let total = d.pages().len();
                    if self.curr_page < total {
                        self.curr_page += 1;
                        return Task::done(Message::RefreshPage);
                    }
                }
            }
            Message::PreviousPage => {
                if self.curr_page > 0 {
                    self.curr_page -= 1;
                    return Task::done(Message::RefreshPage);
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
                    Query::ExportFile(_) => (),
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
            .map(|(i, (k, _))| {
                row![
                    mouse_area(
                        container(text(k))
                            .padding(5)
                            .style(container::bordered_box)
                            .width(Length::FillPortion(1))
                    )
                    .on_press(Message::RenameResource(i)),
                    horizontal().width(25),
                    button("Download").on_press(Message::DownloadResource(i)),
                    button("Delete").on_press(Message::DeleteResource(i)),
                ]
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
                    button("…").on_press(Message::MultiLineDescription)
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
                    button("+").on_press(Message::NewAlternative)
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
                button("Upload").on_press(Message::UploadResource),
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
                button("Edit Template").on_press(Message::EditTemplate),
            ]
        ]
        .spacing(10)
        .width(Length::FillPortion(1))
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
                    button("Export")
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
            .width(Length::FillPortion(1))
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
                                horizontal().width(25),
                                mouse_area(
                                    container(text(&v.value))
                                        .padding(5)
                                        .style(container::bordered_box)
                                        .width(Length::FillPortion(3))
                                )
                                .on_press(Message::StartEdit(i, false)),
                            ]
                        } else {
                            row![
                                mouse_area(
                                    container(text(k))
                                        .padding(5)
                                        .style(container::bordered_box)
                                        .width(Length::FillPortion(1))
                                )
                                .on_press(Message::StartEdit(i, true)),
                                horizontal().width(25),
                                text_input("Value", val)
                                    .id("currentEdit")
                                    .on_input(Message::EditVariableField)
                                    .on_submit(Message::SubmitVariableEdit)
                                    .width(Length::FillPortion(3))
                            ]
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
                            horizontal().width(25),
                            mouse_area(
                                container(text(&v.value))
                                    .padding(5)
                                    .style(container::bordered_box)
                                    .width(Length::FillPortion(3))
                            )
                            .on_press(Message::StartEdit(i, false)),
                        ]
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
                        horizontal().width(25),
                        mouse_area(
                            container(text(&v.value))
                                .padding(5)
                                .style(container::bordered_box)
                                .width(Length::FillPortion(3))
                        )
                        .on_press(Message::StartEdit(i, false)),
                    ]
                    .into()
                })
                .collect()
        };
        vars.push(
            row![
                mouse_area(
                    container(text("..New..").color(Color::WHITE.scale_alpha(0.4)))
                        .padding(5)
                        .style(container::bordered_box)
                        .width(Length::FillPortion(1))
                )
                .on_press(Message::NewVariable),
                horizontal().width(25),
                container(text("..Value..").color(Color::WHITE.scale_alpha(0.4)))
                    .padding(5)
                    .style(container::bordered_box)
                    .width(Length::FillPortion(3))
            ]
            .into(),
        );
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
    modified_by: String,
}

impl Default for Variable {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: String::new(),
            last_modified: Utc::now(),
            modified_by: String::new(),
        }
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
