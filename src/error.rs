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
) -> Result<PagedDocument, String> {
    let builder = eng.world_builder().with_inputs(vars);
    let world = match builder.build() {
        Ok(world) => world,
        Err(err) => return Err(err.to_string()),
    };
    let Warned {
        output,
        warnings: _,
    } = typst::compile(&world);
    comemo::evict(30);
    output.map_err(|e| format_source_error(&e, &DiWrd(world)))
}

#[derive(Default)]
struct TypstError {
    errors: String,
}

impl std::io::Write for TypstError {
    fn write(&mut self, buf: &[u8]) -> Result<usize, std::io::Error> {
        self.errors.push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }
    fn flush(&mut self) -> Result<(), std::io::Error> {
        Ok(())
    }
}

impl termcolor::WriteColor for TypstError {
    // Required methods
    fn supports_color(&self) -> bool {
        false
    }
    // might add support for colors using the Span for rich text in iced later
    fn set_color(&mut self, _spec: &termcolor::ColorSpec) -> std::io::Result<()> {
        Ok(())
    }
    fn reset(&mut self) -> std::io::Result<()> {
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

fn format_source_error(errors: &[SourceDiagnostic], world: &DiWrd) -> String {
    let mut err = TypstError::default();
    _ = emit(
        &mut err,
        world,
        errors,
        typst_kit::diagnostics::DiagnosticFormat::Human,
    );
    err.errors
}
