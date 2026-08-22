use chrono::{DateTime, Utc};
use iced::widget::{
    button, column, image, pick_list, row, space::horizontal, text, text_editor, text_input,
};
use iced::Element;
use iced::{Length, Task};
use indexmap::{map::MutableEntryKey, IndexMap};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::{fs::File, path::Path};
use typst_as_lib::{TypstEngine, TypstTemplateMainFile};
use typst_layout::PagedDocument;
use zip::read::ZipArchive;
use zip::write::FileOptions;
use zip::ZipWriter;

static FONT: &[u8] = include_bytes!("./texgyrecursor-regular.otf");

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
    page: Option<image::Handle>,
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

#[derive(Clone)]
enum Query {
    NewAlternative(String, Option<String>),
    RenameAlternative(usize, String),
    EditTemplate(text_editor::Content),
    EditDescription(text_editor::Content),
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
                row![text_input("Name", &name)
                    .on_input(Message::QueryStringEdit)
                    .on_submit(Message::SubmitQuery)]
                .into(),
            ),
            Query::EditTemplate(content) => (
                "Edit Typst Template",
                text_editor(&content)
                    .on_action(Message::QueryEditorAction)
                    .highlight("typ", iced_highlighter::Theme::InspiredGitHub)
                    .height(Length::Fill)
                    .into(),
            ),
            Query::EditDescription(content) => (
                "Edit Description",
                text_editor(&content)
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
    QueryEditorAction(text_editor::Action),
    QueryStringEdit(String),
    QueryDropDownChanged(String),
    ProjectNameChange(String),
    ProjectDescriptionChange(String),
    MultiLineDescription,
    NewAlternative,
    AlternativeSelect(String),
    EditVariableName(usize, String),
    EditVariableValue(usize, String),
    NewVariable(String),
}

struct TypDoc;

impl State {
    fn name() -> &'static str {
        "TypDoc"
    }

    fn boot(&self) -> (State, Task<Message>) {
        (State::default(), Task::none())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::OpenProject => {
                self.project = Project::from_file("test.typdoc".as_ref()).unwrap();
            }
            Message::SaveProject => {
                self.project.save_file("test.typdoc".as_ref()).unwrap();
            }
            Message::RefreshPage => {
                println!("{}", self.project.template);
                let eng = self.project.typst_engine();
                self.document = match eng.compile().output {
                    Ok(s) => Some(s),
                    Err(e) => {
                        eprintln!("Error compiling: {e:?}");
                        None
                    }
                };
                if let Some(doc) = &self.document {
                    let opt = typst_render::RenderOptions::default();
                    eprintln!("Pages: {}", doc.pages().len());
                    let pixmap = typst_render::render(&doc.pages()[0], &opt);
                    pixmap.save_png("test.png").unwrap();
                    self.page = Some(image::Handle::from_rgba(
                        pixmap.width(),
                        pixmap.height(),
                        pixmap.data().to_vec(),
                    ));
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
            Message::NewAlternative => {
                self.query = Some(Query::NewAlternative(String::new(), None));
            }
            Message::MultiLineDescription => {
                self.query = Some(Query::EditDescription(text_editor::Content::with_text(
                    &self.project.metadata.description,
                )));
            }
            Message::EditTemplate => {
                self.query = Some(Query::EditTemplate(text_editor::Content::with_text(
                    &self.project.template,
                )));
            }
            Message::AlternativeSelect(alt) => {
                self.alternative = Some(alt);
            }
            Message::EditVariableName(ind, name) => {}
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
                .on_input_maybe(
                    (!self.project.metadata.description.contains("\n"))
                        .then_some(Message::ProjectDescriptionChange)
                )
                .width(Length::FillPortion(3)),
                button("...").on_press(Message::MultiLineDescription)
            ]
            .width(Length::Fill),
            row![
                text("Alternative:"),
                pick_list(
                    self.project.alt_list.as_ref(),
                    self.alternative.as_ref(),
                    Message::AlternativeSelect
                ),
                button("+").on_press(Message::NewAlternative),
            ]
            .width(Length::Fill),
            button("Edit Template").on_press(Message::EditTemplate),
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
                    button("Export")
                ],
                self.page()
            ]
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
    variables: IndexMap<String, String>,
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
            .fonts([FONT])
            .build()
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
struct Resource {
    path: PathBuf,
    #[serde(skip)]
    contents: Vec<u8>,
    last_modfied: DateTime<Utc>,
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
    last_modifed: DateTime<Utc>,
    in_template: bool,
}
