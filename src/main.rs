use chrono::{DateTime, Utc};
use iced::widget::{
    button, column, image, pick_list, row, space::horizontal, text, text_editor, text_input,
};
use iced::Element;
use iced::{Length, Task};
use indexmap::{map::MutableEntryKey, IndexMap};
use std::path::PathBuf;

fn main() {
    iced::application(State::default, State::update, State::view)
        .run()
        .unwrap()
}

#[derive(Default, Clone)]
struct State {
    project: Project,
    alternative: Option<String>,
    page: Option<image::Handle>,
    query: Option<Query>,
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
        let q: Element<_> = match self {
            Query::NewAlternative(name, from) => row![
                text_input("Name", &name).on_input(Message::QueryStringEdit),
                pick_list(alts, from.as_ref(), Message::QueryDropDownChanged),
            ]
            .into(),
            Query::RenameAlternative(_, name) => row![text_input("Name", &name)
                .on_input(Message::QueryStringEdit)
                .on_submit(Message::SubmitQuery)]
            .into(),
            Query::EditTemplate(content) => text_editor(&content)
                .on_action(Message::QueryEditorAction)
                .highlight("typ", iced_highlighter::Theme::InspiredGitHub)
                .height(Length::Fill)
                .into(),
            Query::EditDescription(content) => text_editor(&content)
                .on_action(Message::QueryEditorAction)
                .height(Length::Fill)
                .into(),
            Query::ExportFile(_) => text("todo").into(),
        };
        column![
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
    NewAlternativeName(String),
    NewAlternativeFrom(String),
    NewAlternativeSubmit,
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
            Message::SubmitQuery => {
                let q = match self.query.take() {
                    Some(q) => q,
                    // invalid state
                    None => return Task::none(),
                };
                match q {
                    Query::NewAlternative(name, from) => {
                        self.project.add_alternative(name, from);
                    }
                    Query::RenameAlternative(ind, name) => {
                        self.project.rename_alternative(ind, name);
                    }
                    Query::EditTemplate(content) => {
                        self.project.template.content = content.text();
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
            Message::NewAlternativeName(name) => {
                if let Some(Query::NewAlternative(q, _f)) = &mut self.query {
                    *q = name;
                }
            }
            Message::NewAlternativeFrom(from) => {
                if let Some(Query::NewAlternative(_q, f)) = &mut self.query {
                    *f = Some(from);
                }
            }
            Message::NewAlternativeSubmit => {
                if let Some(Query::NewAlternative(q, _f)) = &self.query {
                    self.project
                        .alternatives
                        .insert(q.to_string(), Alternative::new(q.to_string()));
                }
            }
            Message::MultiLineDescription => {
                self.query = Some(Query::EditDescription(text_editor::Content::with_text(
                    &self.project.metadata.description,
                )));
            }
            Message::EditTemplate => {
                self.query = Some(Query::EditTemplate(text_editor::Content::with_text(
                    &self.project.template.content,
                )));
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
                text("Project Name:").width(Length::FillPortion(1)),
                text_input("Name", &self.project.metadata.name)
                    .on_input(Message::ProjectNameChange)
                    .width(Length::FillPortion(3))
            ]
            .width(Length::Fill),
            row![
                text("Description:").width(Length::FillPortion(1)),
                text_input("Description", &self.project.metadata.description)
                    .on_input(Message::ProjectDescriptionChange)
                    .width(Length::FillPortion(3)),
                button("...").on_press(Message::MultiLineDescription)
            ]
            .width(Length::Fill),
            row![
                text("Alternative:"),
                text("Placeholder"),
                button("Add new").on_press(Message::NewAlternative),
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
            column![row![button("Refresh"), button("Export")], self.page()]
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

#[derive(Default, Clone)]
struct Project {
    metadata: Metadata,
    template: Template,
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
}

#[derive(Default, Clone)]
struct Resource {
    path: PathBuf,
    contents: Vec<u8>,
    last_modfied: DateTime<Utc>,
}

#[derive(Default, Clone)]
struct Template {
    /// Actual typst content
    content: String,
    /// When was the content last modified
    last_modified: DateTime<Utc>,
    /// The user that last modified the template
    modified_by: String,
}

#[derive(Default, Clone)]
struct Metadata {
    name: String,
    description: String,
    last_modified: DateTime<Utc>,
}

#[derive(Clone)]
struct Alternative {
    name: String,
    variables: IndexMap<String, String>,
}

impl Alternative {
    fn new(name: String) -> Alternative {
        Alternative {
            name,
            variables: IndexMap::new(),
        }
    }
}

#[derive(Clone)]
struct Variable {
    name: String,
    value: String,
    last_modifed: DateTime<Utc>,
    in_template: bool,
}
