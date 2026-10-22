use iced::widget::text::Span;
use typst::diag::{SourceDiagnostic, Warned};
use typst::foundations::{Dict, Value};
use typst::syntax::FileId;
use typst::utils::LazyHash;
use typst_as_lib::{TypstAsLibError, TypstEngine, TypstTemplateMainFile, TypstWorld};
use typst_kit::diagnostics::{DiagnosticWorld, emit};
use typst_layout::PagedDocument;

pub fn compile_typst(
    eng: &TypstEngine<TypstTemplateMainFile>,
    vars: Dict,
) -> Result<PagedDocument, TypstError> {
    let builder = eng.world_builder().with_inputs(vars);
    let world = match builder.build() {
        Ok(world) => world,
        Err(err) => {
            return Err(TypstError {
                color: None,
                errors: vec![Span::new(err.to_string())],
            });
        }
    };
    let Warned {
        output,
        warnings: _,
    } = typst::compile(&world);
    comemo::evict(30);
    output.map_err(|e| format_source_error(&e, &DiWrd(world)))
}

#[derive(Default)]
pub struct TypstError {
    color: Option<iced::Color>,
    pub errors: Vec<Span<'static>>,
}

impl std::io::Write for TypstError {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        let txt = String::from_utf8_lossy(buf).to_string();
        let span = if let Some(col) = &self.color {
            Span::new(txt).color(col.clone())
        } else {
            Span::new(txt)
        };
        self.errors.push(span);
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

impl termcolor::WriteColor for TypstError {
    fn supports_color(&self) -> bool {
        true
    }

    fn set_color(&mut self, spec: &termcolor::ColorSpec) -> std::io::Result<()> {
        if let Some(fg) = spec.fg() {
            self.color = match fg {
                termcolor::Color::Black => Some(iced::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                }),
                termcolor::Color::Blue => Some(iced::Color {
                    r: 0.0,
                    g: 0.0,
                    b: 1.0,
                    a: 1.0,
                }),
                termcolor::Color::Green => Some(iced::Color {
                    r: 0.0,
                    g: 1.0,
                    b: 0.0,
                    a: 1.0,
                }),
                termcolor::Color::Red => Some(iced::Color {
                    r: 1.0,
                    g: 0.0,
                    b: 0.0,
                    a: 1.0,
                }),
                termcolor::Color::Cyan => Some(iced::Color {
                    r: 0.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                }),
                termcolor::Color::Magenta => Some(iced::Color {
                    r: 1.0,
                    g: 0.0,
                    b: 1.0,
                    a: 1.0,
                }),
                termcolor::Color::Yellow => Some(iced::Color {
                    r: 1.0,
                    g: 1.0,
                    b: 0.0,
                    a: 1.0,
                }),
                termcolor::Color::White => Some(iced::Color {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                }),
                termcolor::Color::Ansi256(_x) => None,
                termcolor::Color::Rgb(r, g, b) => Some(iced::Color {
                    r: *r as f32,
                    g: *g as f32,
                    b: *b as f32,
                    a: 1.0,
                }),
                _ => None,
            };
        }
        Ok(())
    }
    fn reset(&mut self) -> std::io::Result<()> {
        self.color = None;
        Ok(())
    }
}

struct DiWrd<'a>(TypstWorld<'a>);

impl<'a> typst::World for DiWrd<'a> {
    // Required methods
    fn library(&self) -> &LazyHash<typst::Library> {
        self.0.library()
    }
    fn book(&self) -> &LazyHash<typst::text::FontBook> {
        self.0.book()
    }
    fn main(&self) -> FileId {
        self.0.main()
    }
    fn source(&self, id: FileId) -> typst::diag::FileResult<typst::syntax::Source> {
        self.0.source(id)
    }
    fn file(&self, id: FileId) -> typst::diag::FileResult<typst::foundations::Bytes> {
        self.0.file(id)
    }
    fn font(&self, index: usize) -> Option<typst::text::Font> {
        self.0.font(index)
    }
    fn today(
        &self,
        offset: Option<typst::foundations::Duration>,
    ) -> Option<typst::foundations::Datetime> {
        self.0.today(offset)
    }
}

impl<'a> DiagnosticWorld for DiWrd<'a> {
    fn name(&self, _id: FileId) -> String {
        "file".into()
    }
}

fn format_source_error(errors: &[SourceDiagnostic], world: &DiWrd) -> TypstError {
    let mut err = TypstError::default();
    _ = emit(
        &mut err,
        world,
        errors,
        typst_kit::diagnostics::DiagnosticFormat::Human,
    );
    err
}
