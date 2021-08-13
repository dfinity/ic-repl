use crate::token::{error2, ParserError};
use codespan_reporting::diagnostic::{Diagnostic, Label};
use codespan_reporting::files::SimpleFile;
use codespan_reporting::term::{self, termcolor::StandardStream};

fn report(e: &ParserError) -> Diagnostic<()> {
    use lalrpop_util::ParseError::*;
    let mut diag = Diagnostic::error().with_message("parser error");
    let label = match e {
        User { error } => Label::primary((), error.span.clone()).with_message(&error.err),
        InvalidToken { location } => {
            Label::primary((), *location..location + 1).with_message("Invalid token")
        }
        UnrecognizedEOF { location, expected } => {
            diag = diag.with_notes(report_expected(expected));
            Label::primary((), *location..location + 1).with_message("Unexpected EOF")
        }
        UnrecognizedToken { token, expected } => {
            diag = diag.with_notes(report_expected(expected));
            Label::primary((), token.0..token.2).with_message("Unexpected token")
        }
        ExtraToken { token } => Label::primary((), token.0..token.2).with_message("Extra token"),
    };
    diag.with_labels(vec![label])
}

fn report_expected(expected: &[String]) -> Vec<String> {
    if expected.is_empty() {
        return Vec::new();
    }
    use pretty::RcDoc;
    let doc: RcDoc<()> = RcDoc::intersperse(
        expected.iter().map(RcDoc::text),
        RcDoc::text(",").append(RcDoc::softline()),
    );
    let header = if expected.len() == 1 {
        "Expects"
    } else {
        "Expects one of"
    };
    let doc = RcDoc::text(header).append(RcDoc::softline().append(doc));
    vec![doc.pretty(70).to_string()]
}

pub fn pretty_parse<T>(name: &str, str: &str) -> Result<T, ParserError>
where
    T: std::str::FromStr<Err = ParserError>,
{
    let str = shellexpand::env(str).map_err(|e| error2(e, 0..0))?;
    str.parse::<T>().map_err(|e| {
        let writer = StandardStream::stderr(term::termcolor::ColorChoice::Auto);
        let config = term::Config::default();
        let file = SimpleFile::new(name, str);
        term::emit(&mut writer.lock(), &config, &file, &report(&e)).unwrap();
        e
    })
}
