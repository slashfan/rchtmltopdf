//! Running a conversion.
//!
//! Everything above this has turned a command line into settings; everything
//! below takes settings and knows nothing about how they were written. This is
//! the seam, and it is short on purpose.
//!
//! # Several documents
//!
//! Each is loaded and printed on its own page of one browser, in command line
//! order, and the printed documents are combined afterwards (#36). One browser
//! rather than one per document (D11), restarted only when a document asks for
//! something that is decided on the browser's command line rather than over the
//! protocol: a proxy, a minimum font size, images off. Two documents that share
//! those share a browser; two that differ each get one, and the result is the
//! same either way.
//!
//! Every input is resolved before any browser starts, so a missing file at the
//! end of a ten document command line fails in a millisecond rather than after
//! nine conversions.
//!
//! # The bands come last
//!
//! Headers and footers are not drawn while a document prints. Nobody knows how
//! many pages it has until it has been laid out, and `[page]` counts across
//! every document of the conversion (#39), so the bands wait: the documents are
//! printed without them and merged, the counts are read, one sheet per page is
//! written with every placeholder expanded, the browser prints that document
//! of sheets, and each sheet is drawn onto its page (D38).
//!
//! One thing about a band comes *first*. A band that is a document
//! (`--header-html`) sizes the margin on its side unless that margin was
//! written, and the print call needs the number, so the document is loaded
//! and measured before the pages it decorates are printed (D39). The sheet
//! then frames it once per page, with the page's numbers as a query string.

use crate::{PROGRAM, VERSION, input, numbering, outline, output, toc};
use rchtmltopdf_browser::Browser;
use rchtmltopdf_browser::LaunchOptions;
use rchtmltopdf_browser::band::{self, Edge, Measured, Sheet};
use rchtmltopdf_browser::clock;
use rchtmltopdf_browser::deadline;
use rchtmltopdf_browser::intercept;
use rchtmltopdf_browser::locate::{SystemEnvironment, locate};
use rchtmltopdf_browser::placeholder;
use rchtmltopdf_browser::plan::{self, Plan};
use rchtmltopdf_browser::render::{Failed, HeadingBox, LoadReport, Progress};
use rchtmltopdf_core::settings::{Band, ObjectKind, ObjectSettings, Settings};
use rchtmltopdf_core::{ExitCode, Input, LoadErrorHandling, NetworkError, is_media_file};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug)]
pub enum ConvertError {
    /// Recognised, and not built yet. Said plainly rather than done partly.
    Unsupported(String),
    /// Every document was dropped by `--load-error-handling skip`, so there is
    /// nothing to write. One entry per document, as `could not load` lines.
    NothingLeft(Vec<String>),
    /// A document never arrived and `--load-error-handling abort` — the default
    /// — ended the conversion. Carries the failure rather than a sentence about
    /// it, so the exit line can name it (D14).
    DocumentFailed(Failed),
    /// `--dump-outline` named a file that could not be written.
    Dump {
        path: PathBuf,
        reason: String,
    },
    Input(input::InputError),
    Output(output::OutputError),
    Browser(rchtmltopdf_browser::Error),
    Pdf(rchtmltopdf_pdf::Error),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::Unsupported(what) => write!(f, "{what}"),
            ConvertError::NothingLeft(failures) => write!(
                f,
                "--load-error-handling skip left nothing to convert: {}",
                failures.join("; ")
            ),
            ConvertError::Dump { path, reason } => {
                write!(
                    f,
                    "could not write the outline to {}: {reason}",
                    path.display()
                )
            }
            ConvertError::DocumentFailed(failed) => {
                write!(f, "could not load {}: {}", failed.url, failed.error.name())
            }
            ConvertError::Input(error) => write!(f, "{error}"),
            ConvertError::Output(error) => write!(f, "{error}"),
            ConvertError::Browser(error) => write!(f, "{error}"),
            ConvertError::Pdf(error) => write!(f, "{error}"),
        }
    }
}

impl ConvertError {
    /// The load failure behind this, if a load is what failed.
    ///
    /// wkhtmltopdf ends such a run with `Exit with code 1 due to network
    /// error: <Name>`, whether the document was missing from the disk or from
    /// the network — it fetched both through the same stack. Applications grep
    /// for that line, so the binary writes it once, for every error that
    /// answers here (D14, D48).
    pub fn network_error(&self) -> Option<NetworkError> {
        match self {
            ConvertError::DocumentFailed(failed) => Some(failed.error),
            ConvertError::Input(error) => Some(error.error),
            _ => None,
        }
    }
}

impl std::error::Error for ConvertError {}

impl From<input::InputError> for ConvertError {
    fn from(error: input::InputError) -> Self {
        ConvertError::Input(error)
    }
}

impl From<output::OutputError> for ConvertError {
    fn from(error: output::OutputError) -> Self {
        ConvertError::Output(error)
    }
}

impl From<rchtmltopdf_pdf::Error> for ConvertError {
    fn from(error: rchtmltopdf_pdf::Error) -> Self {
        ConvertError::Pdf(error)
    }
}

impl From<rchtmltopdf_browser::Error> for ConvertError {
    fn from(error: rchtmltopdf_browser::Error) -> Self {
        ConvertError::Browser(error)
    }
}

/// One object as the browser printed it.
struct Piece {
    pdf: Vec<u8>,
    /// The document's own `<title>`, read from the page once it had settled
    /// rather than off the printed part: Chromium writes the URL into the
    /// Info dictionary of a document that has none (D52). What `[title]`
    /// prints, what the file's title falls back to, and what names the
    /// object's item in `--dump-outline`.
    title: String,
    /// Its headings' boxes, measured before printing when this document's
    /// headings link back to the table of contents (D57). Empty otherwise.
    headings: Vec<HeadingBox>,
}

/// What came out of the browser for the documents that made it.
struct Printed {
    /// One piece per document that was printed, in command line order, with
    /// the index of the object it came from.
    documents: Vec<(usize, Piece)>,
    /// Subresources the interception refused, across every document and band
    /// sheet. Each one is exit 1 whatever the handlers say (D49).
    refused: Vec<intercept::Refusal>,
    /// Subresources that failed, paired with the object they belong to. Its
    /// `--load-media-error-handling` judges the media files among them, and
    /// the exit code judges the rest (D49).
    subresources: Vec<(usize, Vec<Failed>)>,
    /// Documents `--load-error-handling skip` dropped, as `could not load` lines.
    skipped: Vec<String>,
    /// Documents that never arrived and were not aborted on: skipped, or
    /// ignored and left as a blank page. The file is written and the exit code
    /// is still 1 (D44), and the first of these names the error on the line
    /// applications grep for.
    failed: Vec<Failed>,
}

/// One object's band documents, resolved the way its input is.
#[derive(Debug, Default)]
struct BandDocuments {
    header: Option<input::Resolved>,
    footer: Option<input::Resolved>,
}

impl BandDocuments {
    fn resolve(object: &ObjectSettings) -> Result<Self, input::InputError> {
        let resolve = |band: &Band| -> Result<Option<input::Resolved>, input::InputError> {
            band.html
                .as_deref()
                .map(|written| input::resolve(&Input::classify(written)))
                .transpose()
        };
        Ok(Self {
            header: resolve(&object.header)?,
            footer: resolve(&object.footer)?,
        })
    }

    fn urls(&self) -> impl Iterator<Item = &str> {
        [&self.header, &self.footer]
            .into_iter()
            .flatten()
            .map(input::Resolved::url)
    }
}

/// What one object's band documents measured. Zero where there is none.
#[derive(Debug, Clone, Copy, Default)]
struct BandHeights {
    header: Measured,
    footer: Measured,
}

/// Everything that goes into the finished file, in the order it was written.
///
/// The printed documents come out of the loop in order, with any that
/// `--load-error-handling skip` dropped simply missing; the tables of contents
/// are slotted back in at the positions they were written on.
fn ordered<'a>(
    printed: &'a [(usize, Piece)],
    tables: &'a [(usize, Piece)],
) -> Vec<(usize, &'a Piece)> {
    let mut all: Vec<(usize, &Piece)> = printed
        .iter()
        .chain(tables)
        .map(|(index, piece)| (*index, piece))
        .collect();
    all.sort_by_key(|(index, _)| *index);
    all
}

/// What one round of building the tables of contents needs to know.
struct ContentsJob<'a> {
    objects: &'a [&'a ObjectSettings],
    plans: &'a [Plan],
    documents: &'a [input::Resolved],
    /// The documents that printed, in order, with their object index.
    printed: &'a [(usize, Piece)],
    /// How many pages each of those contributed to the merge.
    counts: &'a [usize],
    /// Which objects are tables of contents.
    tables: &'a [usize],
    /// The headings of the merge, to be listed.
    headings: &'a [rchtmltopdf_pdf::OutlineItem],
}

/// How many times a table of contents is built before its length is taken as
/// settled.
///
/// The length usually settles on the first go and always on the second: what
/// can move it is a number growing a digit and wrapping a line. The bound is
/// here so that a pathological document costs a few prints rather than a
/// conversion that never ends.
const CONTENTS_PASSES: usize = 4;

/// Build and print every table of contents, until its length stops changing.
///
/// **Why it is a loop.** A table lists the pages of the documents behind it,
/// and its own pages push those documents down, so the numbers depend on the
/// length and the length can depend on the numbers. wkhtmltopdf settles the
/// same way, and settles exactly: with a three-page table, the first heading
/// behind it is numbered 4.
async fn contents(
    job: ContentsJob<'_>,
    browser: &Browser,
    progress: &Progress,
) -> Result<Vec<(usize, Piece)>, ConvertError> {
    // Where each printed document started in the merge the headings were read
    // off, which is what carries a heading from that merge to the finished file.
    let mut merged_first: HashMap<usize, usize> = HashMap::new();
    let mut running = 1usize;
    for ((index, _), pages) in job.printed.iter().zip(job.counts) {
        merged_first.insert(*index, running);
        running += pages;
    }

    let mut lengths: Vec<usize> = vec![1; job.tables.len()];
    let mut printed: Vec<(usize, Piece)> = Vec::new();
    for _ in 0..CONTENTS_PASSES {
        // Where everything lands, with the tables at the length last measured.
        let mut pieces: Vec<(usize, usize)> = job
            .printed
            .iter()
            .zip(job.counts)
            .map(|((index, _), pages)| (*index, *pages))
            .chain(
                job.tables
                    .iter()
                    .zip(&lengths)
                    .map(|(index, pages)| (*index, *pages)),
            )
            .collect();
        pieces.sort_by_key(|(index, _)| *index);

        let mut first_page: HashMap<usize, usize> = HashMap::new();
        let mut running = 1usize;
        for (index, pages) in &pieces {
            first_page.insert(*index, running);
            running += pages;
        }

        // The same numbering the outline dump uses, over the finished layout.
        // The offset is the conversion's, so any plan's copy is every plan's
        // (D51).
        let offset = job
            .plans
            .first()
            .map_or(0, |plan| plan.finish.numbering.page_offset);
        let where_tables_are: Vec<(usize, &str)> = job
            .tables
            .iter()
            .map(|index| {
                (
                    first_page[index],
                    job.objects[*index].toc.header_text.as_str(),
                )
            })
            .collect();

        printed.clear();
        for index in job.tables {
            let entries = toc::entries(
                job.headings,
                |page| moved(page, &merged_first, &first_page, job.printed, job.counts),
                &where_tables_are,
                |page| numbering::dump_page(page, offset),
            );
            let settings = job.plans[*index]
                .toc
                .as_ref()
                .expect("a table of contents carries its own settings");
            let document = &job.documents[*index];
            let path = document
                .scratch_path()
                .expect("a table of contents is written to a file of ours");
            std::fs::write(path, toc::document(&entries, settings)).map_err(|error| {
                ConvertError::Unsupported(format!("could not write the table of contents: {error}"))
            })?;

            let page = browser.new_page().await?;
            page.prepare(&job.plans[*index].prepare).await?;
            let report = page
                .load(document.url(), &job.objects[*index].load, progress)
                .await?;
            if let Some(failed) = &report.document {
                return Err(rchtmltopdf_browser::Error::Navigation {
                    url: failed.url.clone(),
                    reason: format!(
                        "{} (the table of contents, which is written by this program)",
                        failed.error.name()
                    ),
                }
                .into());
            }
            let headings = match job.plans[*index].finish.back_links {
                true => page.heading_boxes().await?,
                false => Vec::new(),
            };
            printed.push((
                *index,
                Piece {
                    pdf: page.print_to_pdf(&job.plans[*index].print).await?,
                    title: report.title,
                    headings,
                },
            ));
        }

        let measured: Vec<usize> = printed
            .iter()
            .map(|(_, piece)| {
                rchtmltopdf_pdf::merge(&[rchtmltopdf_pdf::Part {
                    pdf: &piece.pdf,
                    url: "",
                    links: &rchtmltopdf_core::settings::LinkSettings::default(),
                    contents: true,
                }])
                .map(|merged| merged.pages[0])
            })
            .collect::<Result<_, _>>()?;
        if measured == lengths {
            break;
        }
        lengths = measured;
    }
    Ok(printed)
}

/// Carry a page of the merge the headings were read off to the page it landed
/// on once the tables of contents were inserted.
fn moved(
    page: usize,
    merged_first: &HashMap<usize, usize>,
    first_page: &HashMap<usize, usize>,
    printed: &[(usize, Piece)],
    counts: &[usize],
) -> usize {
    let mut running = 1usize;
    for ((index, _), pages) in printed.iter().zip(counts) {
        if page < running + pages {
            return first_page[index] + (page - merged_first[index]);
        }
        running += pages;
    }
    page
}

/// The back links of one document: its headings in the finished file, each
/// with the box measured for it before printing (D57).
///
/// `items` is the whole outline with the file's page numbers, and the
/// document's headings are the top-level entries on its pages, walked in
/// reading order, which is the order the boxes were measured in. The two
/// are paired by title, advancing through the boxes: a heading Chromium left
/// out of the outline is skipped over, and one the measurement did not see
/// gets no back link rather than another heading's box.
fn back_links_of(
    items: &[rchtmltopdf_pdf::OutlineItem],
    pages: std::ops::RangeInclusive<usize>,
    boxes: &[HeadingBox],
    scale: f64,
    right: f64,
) -> Vec<rchtmltopdf_pdf::BackLink> {
    fn walk<'a>(
        items: &'a [rchtmltopdf_pdf::OutlineItem],
        out: &mut Vec<&'a rchtmltopdf_pdf::OutlineItem>,
    ) {
        for item in items {
            out.push(item);
            walk(&item.children, out);
        }
    }
    let own: Vec<rchtmltopdf_pdf::OutlineItem> = items
        .iter()
        .filter(|item| pages.contains(&item.page))
        .cloned()
        .collect();
    let mut headings = Vec::new();
    walk(&own, &mut headings);
    let collapse = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");

    let mut next = 0usize;
    let mut out = Vec::new();
    for heading in headings {
        let title = collapse(&heading.title);
        let Some(found) = boxes[next.min(boxes.len())..]
            .iter()
            .position(|measured| collapse(&measured.title) == title)
        else {
            continue;
        };
        let measured = &boxes[next + found];
        next += found + 1;
        out.push(rchtmltopdf_pdf::BackLink {
            page: heading.page,
            left: heading.left,
            top: heading.top,
            right,
            height: measured.height * scale,
        });
    }
    out
}

/// Convert, or say why not.
///
/// Returns an exit code rather than nothing, because "wrote the document and
/// still failed" is a real outcome and D14 turns on it: a subresource that fails
/// under `--load-media-error-handling abort` produces the PDF *and* exits 1.
/// Collapsing that into `Err` would throw the document away, and collapsing it
/// into `Ok` would lose the exit code KnpSnappy reads.
pub async fn convert(settings: &Settings) -> Result<ExitCode, ConvertError> {
    let objects = pages(settings)?;

    // Resolved before a browser is started, so a missing file fails in a
    // millisecond rather than after a launch. Held for the whole conversion: a
    // document read from standard input lives in a file that goes away when this
    // is dropped, on every path out including the deadline.
    let mut documents = Vec::with_capacity(objects.len());
    for (position, object) in objects.iter().enumerate() {
        documents.push(match object.kind {
            // A table of contents has no input to read: it is written here,
            // empty for now, and written again with the real entries once the
            // pages it lists have been counted (D41). The path stays the same
            // across both, so the plan built from it below is the plan that
            // prints it.
            ObjectKind::Toc => input::scratch_document(
                &format!("toc-{position}"),
                &toc::document(&[], &object.toc),
            )?,
            ObjectKind::Page | ObjectKind::Cover => {
                let source = object.input.as_ref().ok_or_else(|| {
                    ConvertError::Unsupported("this object has no document to read".into())
                })?;
                // A file that is not there is a load that failed, and the
                // handler says what becomes of it (D55): under `abort` that
                // is the end, so it is said now rather than after a launch;
                // under the other two the document is carried to the loop,
                // which drops it or leaves a blank page in its place (D44).
                match object.load.on_document_error {
                    LoadErrorHandling::Abort => input::resolve(source)?,
                    LoadErrorHandling::Skip | LoadErrorHandling::Ignore => {
                        input::resolve_or_report(source)?
                    }
                }
            }
        });
    }

    // Their positions among the objects, which is where their pages go.
    let tables: Vec<usize> = objects
        .iter()
        .enumerate()
        .filter(|(_, object)| object.kind == ObjectKind::Toc)
        .map(|(position, _)| position)
        .collect();

    // The band documents too, and for the same reason: `--header-html` names
    // a file the way the input does, and a missing one fails now.
    let mut band_documents = Vec::with_capacity(objects.len());
    for object in &objects {
        band_documents.push(BandDocuments::resolve(object)?);
    }
    // The sheets are one page for every object's bands, so one rule and one
    // wait serve all of them.
    let band_policy = plan::band_policy(objects.iter().copied());
    let band_delay = objects
        .iter()
        .filter(|object| object.header.html.is_some() || object.footer.html.is_some())
        .map(|object| object.load.javascript_delay)
        .max()
        .unwrap_or(Duration::ZERO);
    let band_web = objects
        .iter()
        .find(|object| object.header.html.is_some() || object.footer.html.is_some())
        .map(|object| &object.web);

    let executable = locate(settings.global.browser.path.as_deref(), &SystemEnvironment)?;

    // Everything the settings decide, decided in one place before any of it
    // happens. The page halves are rebuilt from the same functions below rather
    // than passed down, so a test can hold the option table to what a conversion
    // would actually do (D27).
    //
    // One clock for every document: a `[date]` in the first footer and one in
    // the last must agree, whatever midnight does in between.
    let now = clock::now();
    let plans: Vec<Plan> = objects
        .iter()
        .zip(&documents)
        .map(|(object, document)| Plan::new(&settings.global, object, now, document.url()))
        .collect();

    // Facts about the command line and the documents rather than about the
    // rendering, so they are said before a browser starts. Once each: ten
    // documents with the same `--encoding` are one warning, not ten.
    if settings.global.log_level.shows_warnings() {
        let mut said: Vec<String> = Vec::new();
        let mut warn = |line: String| {
            if !said.contains(&line) {
                eprintln!("{PROGRAM}: warning: {line}");
                said.push(line);
            }
        };
        for ((object, document), plan) in objects.iter().zip(&documents).zip(&plans) {
            if object.web.encoding.is_some() && plan.requests.serve_as.is_none() {
                warn(
                    "--encoding does not apply to a document fetched over the network; it is \
                     read as the server said it should be"
                        .into(),
                );
            }
            if !object.web.cookies.is_empty() && !plan::takes_cookies(document.url()) {
                warn(
                    "--cookie needs a document fetched over http or https; a local document \
                     has no origin to scope a cookie to, so it is being ignored"
                        .into(),
                );
            }
        }
    }

    let progress = Progress::new();
    let total = objects.len();
    // The browser is created inside the deadline, so expiry drops it and its Drop
    // stops the process group and removes the profile. Cleanup is not a step that
    // could be skipped. The merge and the bands are inside it too: the bands
    // need the browser, and D16 bounds the whole conversion.
    let (pdf, printed, file_title) = deadline::within(settings.global.timeout, &progress, async {
        let mut printed = Printed {
            documents: Vec::with_capacity(total),
            refused: Vec::new(),
            subresources: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
        };
        let mut browser: Option<Browser> = None;
        let mut running_with: Option<LaunchOptions> = None;
        // Measured once per document, whichever objects share it.
        let mut measured: HashMap<String, Measured> = HashMap::new();
        let mut heights: Vec<BandHeights> = vec![BandHeights::default(); total];

        for (index, ((object, document), plan)) in
            objects.iter().zip(&documents).zip(&plans).enumerate()
        {
            // Printed after the merge, when there are pages to list (D41).
            if object.kind == ObjectKind::Toc {
                continue;
            }

            // One browser for the conversion, restarted only when this document
            // needs one started differently. Closed rather than dropped, so it
            // can finish writing its profile away.
            if running_with.as_ref() != Some(&plan.launch) {
                if let Some(previous) = browser.take() {
                    previous.close().await?;
                }
                browser = Some(Browser::launch(&executable, &plan.launch).await?);
                running_with = Some(plan.launch.clone());
            }
            let browser = browser.as_ref().expect("launched just above");

            // Before the pages, because the print call needs the numbers: a
            // band document's height is its margin unless the margin was
            // written (D39).
            let page_setup = &settings.global.page;
            let (mut header_height, mut footer_height) = (Measured::default(), Measured::default());
            for (band_document, slot) in [
                (&band_documents[index].header, &mut header_height),
                (&band_documents[index].footer, &mut footer_height),
            ] {
                let Some(band_document) = band_document else {
                    continue;
                };
                let url = band_document.url();
                *slot = match measured.get(url) {
                    Some(known) => *known,
                    None => {
                        let value = measure(
                            browser,
                            url,
                            &plan::measure_prepare(page_setup, &object.web),
                            &plan::band_load(object.load.javascript_delay),
                            plan::band_rules(&band_policy, url, band_documents[index].urls()),
                            &progress,
                        )
                        .await?;
                        measured.insert(url.to_string(), value);
                        value
                    }
                };
            }
            heights[index] = BandHeights {
                header: header_height,
                footer: footer_height,
            };
            let print = plan::reserve(
                &plan.print,
                reserved_inches(
                    &object.header,
                    page_setup.named.top,
                    &heights[index].header,
                    plan::zoom(&object.web),
                ),
                reserved_inches(
                    &object.footer,
                    page_setup.named.bottom,
                    &heights[index].footer,
                    plan::zoom(&object.web),
                ),
            );

            say(settings, &loading_line(index, total));
            let page = browser.new_page().await?;

            // Before anything is fetched, the document included: the policy has
            // to be in place for the first request, not the second (D10).
            let policing = intercept::install(page.session(), plan.requests.clone()).await?;

            page.prepare(&plan.prepare).await?;
            let report = match document.missing() {
                // Never sent to the browser: the file is not there to fetch,
                // and the report says so the way a navigation that failed
                // would (D55), so the handler below judges both alike.
                Some(error) => LoadReport {
                    navigation_failed: true,
                    document: Some(Failed {
                        url: document.url().to_string(),
                        error: error.error,
                        http_status: 0,
                    }),
                    ..LoadReport::default()
                },
                None => page.load(document.url(), &object.load, &progress).await?,
            };

            // Before printing, not after. A rejected password leaves the
            // server's own 401 body as the response, which renders perfectly
            // well and is not the document anybody asked for (D14).
            if policing
                .as_ref()
                .is_some_and(rchtmltopdf_browser::Interception::credentials_rejected)
            {
                return Err(rchtmltopdf_browser::Error::Credentials {
                    url: document.url().to_string(),
                }
                .into());
            }

            // The document itself. `--load-error-handling` decides (D14, D44).
            // Only `abort` ends the conversion; the other two write the file
            // from what did load, and all three exit 1 when the document never
            // arrived at all.
            if let Some(failed) = &report.document {
                // The request itself, before the handler says what to do about
                // it. wkhtmltopdf writes this line whichever handler is in
                // force, and it is the one that names the URL under all three
                // (#114): a script watching stderr finds the address that
                // failed here, and the codes tell a server that refused from a
                // server that was never reached.
                report_failed_request(settings, failed);
                match object.load.on_document_error {
                    LoadErrorHandling::Abort => {
                        return Err(ConvertError::DocumentFailed(failed.clone()));
                    }
                    // `skip` drops the failing document and carries on with the
                    // others. Said now rather than at the end, in wkhtmltopdf's
                    // words, so the line sits next to the load it belongs to.
                    LoadErrorHandling::Skip => {
                        let line =
                            format!("could not load {}: {}", failed.url, failed.error.name());
                        if settings.global.log_level.shows_warnings() {
                            eprintln!(
                                "{PROGRAM}: warning: failed loading page {} (skipped)",
                                failed.url
                            );
                        }
                        if report.navigation_failed {
                            printed.failed.push(failed.clone());
                        }
                        printed.skipped.push(line);
                        continue;
                    }
                    // `ignore` prints whatever did load, which for a 404 is the
                    // server's own error page. Where nothing arrived there is
                    // nothing to print, and wkhtmltopdf left a blank page in
                    // the document's place rather than dropping it: the page
                    // behind it keeps its number (D44).
                    LoadErrorHandling::Ignore => {
                        if report.navigation_failed {
                            if settings.global.log_level.shows_warnings() {
                                eprintln!(
                                    "{PROGRAM}: warning: failed loading page {} (ignored)",
                                    failed.url
                                );
                            }
                            printed.failed.push(failed.clone());
                            page.blank().await?;
                        }
                    }
                }
            }

            // The headings' boxes, for their back links (D57): read now,
            // while the document is still on screen, and only when asked.
            let headings = match plan.finish.back_links && !report.navigation_failed {
                true => page.heading_boxes().await?,
                false => Vec::new(),
            };
            printed.documents.push((
                index,
                Piece {
                    pdf: page.print_to_pdf(&print).await?,
                    title: report.title.clone(),
                    headings,
                },
            ));

            // Read before the guard is dropped, which is what stops interception.
            printed.refused.extend(
                policing
                    .as_ref()
                    .map(intercept::Interception::refused)
                    .unwrap_or_default(),
            );
            printed.subresources.push((index, report.subresources));
        }

        if printed.documents.is_empty() && tables.is_empty() {
            return Err(ConvertError::NothingLeft(printed.skipped));
        }

        say(settings, "Printing pages (2/2)");

        // The documents, merged without the tables of contents: one of those
        // lists the pages around it, and nobody knows how long it is until it
        // has been laid out, so it is built from this merge and inserted into
        // another (D41).
        let content: Vec<rchtmltopdf_pdf::Part<'_>> = printed
            .documents
            .iter()
            .map(|(index, piece)| rchtmltopdf_pdf::Part {
                pdf: &piece.pdf,
                url: &plans[*index].finish.document_url,
                links: &plans[*index].finish.links,
                contents: false,
            })
            .collect();

        let tables_printed: Vec<(usize, Piece)> = if tables.is_empty() {
            Vec::new()
        } else {
            let merged = match content.as_slice() {
                [] => None,
                parts => Some(rchtmltopdf_pdf::merge(parts)?),
            };
            let counts: Vec<usize> = merged
                .as_ref()
                .map(|merged| merged.pages.clone())
                .unwrap_or_default();
            // The headings to list. Generated whatever `--no-outline` says,
            // and every one of them whatever `--outline-depth` says: both
            // options are about the bookmarks the file carries (D53), and a
            // table of contents was asked for separately.
            let headings = match &merged {
                Some(merged) => {
                    rchtmltopdf_pdf::outline(
                        &merged.pdf,
                        &rchtmltopdf_pdf::OutlineTreatment {
                            keep: true,
                            depth: plans[0].finish.outline.depth,
                        },
                    )?
                    .1
                }
                None => Vec::new(),
            };

            let browser = match browser.as_ref() {
                Some(running) => running,
                // Nothing else was printed: `rchtmltopdf toc out.pdf` is a
                // table of contents of nothing, and wkhtmltopdf prints the
                // page.
                // Nothing follows this, so `running_with` is not updated:
                // there is no next document to compare it against.
                None => {
                    browser = Some(Browser::launch(&executable, &plans[tables[0]].launch).await?);
                    browser.as_ref().expect("launched just above")
                }
            };
            contents(
                ContentsJob {
                    objects: &objects,
                    plans: &plans,
                    documents: &documents,
                    printed: &printed.documents,
                    counts: &counts,
                    tables: &tables,
                    headings: &headings,
                },
                browser,
                &progress,
            )
            .await?
        };

        // Everything in the order it was written, the tables among the rest.
        let ordered = ordered(&printed.documents, &tables_printed);
        let parts: Vec<rchtmltopdf_pdf::Part<'_>> = ordered
            .iter()
            .map(|(index, piece)| rchtmltopdf_pdf::Part {
                pdf: &piece.pdf,
                url: &plans[*index].finish.document_url,
                links: &plans[*index].finish.links,
                // The file is named after the first document, and a table of
                // contents is not one: wkhtmltopdf passed over its own.
                contents: tables.contains(index),
            })
            .collect();
        let merged = rchtmltopdf_pdf::merge(&parts)?;

        // `[title]` is the document's own `<title>` and `[doctitle]` the
        // finished file's, and neither was known when the plan was made
        // (#110). Each document said its own once it had loaded (D52); the
        // file's is `--title`, or the first document that is not a table of
        // contents — the same part the merge took its Info dictionary from.
        let file_title = match &settings.global.title {
            Some(given) => given.clone(),
            None => ordered
                .iter()
                .find(|(index, _)| !tables.contains(index))
                .map(|(_, piece)| piece.title.clone())
                .unwrap_or_default(),
        };

        // The outline the browser wrote is all or nothing per document, so the
        // depth is cut here — from the file alone. What comes back is the
        // whole tree: `--outline-depth` bounds the bookmarks and nothing
        // else, so the dump and the bands see every heading (D53). The
        // treatment is global, so the first plan's copy is every plan's.
        let finish = &plans[0].finish;
        // `--page-offset` is one number for the whole output, wherever it was
        // written (D51), so the same copy answers for every page.
        let offset = finish.numbering.page_offset;
        let (pdf, items) = rchtmltopdf_pdf::outline(
            &merged.pdf,
            &rchtmltopdf_pdf::OutlineTreatment {
                keep: finish.outline.enabled,
                depth: finish.outline.depth,
            },
        )?;
        if let Some(path) = &finish.outline.dump {
            // One item per object around its headings, in wkhtmltopdf's
            // shape (D52): a document is named by its `<title>`, a table of
            // contents by its caption, and an object the outline leaves out
            // — a cover, or `--exclude-from-outline` — by nothing at all.
            let dumped: Vec<outline::Object> = ordered
                .iter()
                .zip(&merged.pages)
                .map(|((index, piece), pages)| {
                    let object = objects[*index];
                    let (title, role) = match object.kind {
                        ObjectKind::Toc => {
                            (object.toc.header_text.clone(), outline::Role::Contents)
                        }
                        _ if object.in_outline => (piece.title.clone(), outline::Role::Document),
                        _ => (String::new(), outline::Role::Excluded),
                    };
                    outline::Object {
                        title,
                        pages: *pages,
                        role,
                    }
                })
                .collect();
            let xml = outline::xml(&outline::dump(&dumped, &items, offset));
            std::fs::write(path, xml).map_err(|error| ConvertError::Dump {
                path: path.clone(),
                reason: error.to_string(),
            })?;
        }

        // The bands, now that every count is known (#39): one sheet per page,
        // printed by the browser that printed the pages, drawn onto them (D38).
        let pdf = if ordered
            .iter()
            .any(|(index, _)| !plans[*index].finish.bands.is_empty())
        {
            // `merged.pages` is how many pages each object printed, in the
            // order they were merged: the site frame of `[sitepage]` and
            // `[sitepages]` is cut from it.
            let numbered = numbering::number(&merged.pages, &items, offset);

            let contexts: Vec<placeholder::Context> = ordered
                .iter()
                .map(|(index, piece)| placeholder::Context {
                    title: file_title.clone(),
                    document_title: piece.title.clone(),
                    ..plans[*index].context.clone()
                })
                .collect();

            let (left, right) = (
                settings.global.page.margins.left.to_mm(),
                settings.global.page.margins.right.to_mm(),
            );
            let sheets: Vec<Sheet> = numbered
                .iter()
                .map(|(part, numbers)| {
                    let index = ordered[*part].0;
                    let plan = &plans[index];
                    let context = &contexts[*part];
                    let bands = &plan.finish.bands;
                    // A band that is a document is framed with the page's
                    // numbers as its query string; a band of text is a row.
                    let markup = |band: &Band,
                                  document: &Option<input::Resolved>,
                                  measured: &Measured,
                                  edge: Edge| match document {
                        Some(document) => band::frame(
                            &placeholder::with_query(
                                document.url(),
                                &placeholder::query(context, numbers),
                            ),
                            edge,
                            &settings.global.page,
                            measured,
                            plan::zoom(&objects[index].web),
                        ),
                        None => band::row(band, edge, left, right, context, numbers),
                    };
                    Sheet {
                        header: markup(
                            &bands.header,
                            &band_documents[index].header,
                            &heights[index].header,
                            Edge::Header,
                        ),
                        footer: markup(
                            &bands.footer,
                            &band_documents[index].footer,
                            &heights[index].footer,
                            Edge::Footer,
                        ),
                    }
                })
                .collect();
            let sheet_document =
                input::scratch_document("bands", &band::document(&settings.global.page, &sheets))?;

            let browser = browser
                .as_ref()
                .expect("a document was printed, so a browser is running");
            let page = browser.new_page().await?;
            // The sheets are ours, but what they frame is the user's: a band
            // document reads the disk under the same rule as the input (D10).
            let policing = intercept::install(
                page.session(),
                plan::band_rules(
                    &band_policy,
                    sheet_document.url(),
                    band_documents.iter().flat_map(BandDocuments::urls),
                ),
            )
            .await?;
            page.prepare(&plan::band_prepare(band_web)).await?;
            let report = page
                .load(
                    sheet_document.url(),
                    &plan::band_load(band_delay),
                    &progress,
                )
                .await?;
            if let Some(failed) = &report.document {
                return Err(rchtmltopdf_browser::Error::Navigation {
                    url: failed.url.clone(),
                    reason: format!(
                        "{} (the document of headers and footers, which is written by \
                         this program)",
                        failed.error.name()
                    ),
                }
                .into());
            }
            let sheets_pdf = page
                .print_to_pdf(&plan::band_print(&settings.global.page))
                .await?;
            printed.refused.extend(
                policing
                    .as_ref()
                    .map(intercept::Interception::refused)
                    .unwrap_or_default(),
            );
            rchtmltopdf_pdf::stamp(&pdf, &sheets_pdf)?
        } else {
            pdf
        };

        // Asked to leave rather than killed, so it can finish writing.
        // `--enable-toc-back-links`: an annotation over every heading of a
        // document that asked for it, back to its entry in the table (D57).
        // After the bands, so the pages it annotates are the finished ones.
        let pdf = if ordered
            .iter()
            .any(|(index, _)| plans[*index].finish.back_links)
        {
            let paper = settings.global.page.effective_size();
            let to_points = |mm: f64| mm * 72.0 / 25.4;
            let right = to_points(paper.width.to_mm() - settings.global.page.margins.right.to_mm());
            let mut contents_parts = Vec::new();
            let mut back_links = Vec::new();
            let mut before = 0usize;
            for ((index, piece), pages) in ordered.iter().zip(&merged.pages) {
                let object = objects[*index];
                if object.kind == ObjectKind::Toc {
                    contents_parts.push(rchtmltopdf_pdf::ContentsPart {
                        first_page: before + 1,
                        pages: *pages,
                        keep_links: object.toc.links,
                    });
                }
                if plans[*index].finish.back_links {
                    // Chromium prints CSS pixels at three quarters of a point,
                    // scaled by `--zoom` like everything else on the page.
                    let scale = 0.75 * object.web.zoom;
                    let mut own = back_links_of(
                        &items,
                        before + 1..=before + pages,
                        &piece.headings,
                        scale,
                        right,
                    );
                    // A table's own heading is listed by an entry made before
                    // the table was printed, aimed at the top of its page
                    // (D41): the back link answers to that entry, so it is
                    // aimed the same way.
                    if object.kind == ObjectKind::Toc
                        && let Some(first) = own.first_mut()
                    {
                        first.left = 0.0;
                        first.top = toc::TOP_OF_THE_PAGE;
                    }
                    back_links.extend(own);
                }
                before += pages;
            }
            rchtmltopdf_pdf::contents_back_links(&pdf, &contents_parts, &back_links)?
        } else {
            pdf
        };

        if let Some(browser) = browser {
            browser.close().await?;
        }
        Ok((pdf, printed, file_title))
    })
    .await?;

    // A document that renders with a stylesheet missing is the hardest kind of
    // failure to diagnose, because it renders. Said on stderr, where it cannot
    // corrupt a PDF written to stdout.
    if settings.global.log_level.shows_warnings() {
        for refusal in &printed.refused {
            eprintln!("{PROGRAM}: warning: {refusal}");
        }
    }

    // The print call takes the title from the document's own `<title>` and
    // offers no override, so `--title` can only be honoured by rewriting the
    // file afterwards; and a document with no `<title>` gets the URL from
    // Chromium where wkhtmltopdf wrote nothing, so the title is written
    // whether the option was given or not, and it is the one `[doctitle]`
    // prints (D46, D52). The same pass names the producer and dates the
    // document (#29).
    let pdf = rchtmltopdf_pdf::set_metadata(
        &pdf,
        &rchtmltopdf_pdf::Metadata {
            title: Some(file_title),
            producer: format!("{PROGRAM} {VERSION}"),
            creator: format!("{PROGRAM} {VERSION}"),
            created: now,
        },
    )?;

    // Last, on the finished file: a band that is a document is framed once per
    // page, so the browser emits the header's logo again for every one of them
    // (D62). Sharing them is the only pass here that changes no page — it
    // changes how many objects say the same thing.
    let pdf = rchtmltopdf_pdf::share_repeated_streams(&pdf)?;

    // Written before the media errors are judged, because D14 says a subresource
    // that failed under `abort` produces the document *and* exits 1. A wrapper
    // that raises on the exit code still has the PDF to look at.
    output::write(&settings.global.output, &pdf)?;

    // What failed is sorted the way wkhtmltopdf sorted it (D49). A media file
    // — css, js, png, jpg, jpeg or gif, by the URL's extension — is the
    // business of its object's `--load-media-error-handling`, and only `abort`
    // turns it into an exit code. Anything else that failed is a network error
    // whatever either handler says: a frame's document, a font, an svg, a
    // request with no extension. So is a file the policy refused, on a document
    // or on a band sheet: wkhtmltopdf judged the `about:blank` it swapped in
    // for one, which has no extension either. The first error to decide the
    // exit code is the one named on the line applications grep for.
    let refused: HashSet<&str> = printed
        .refused
        .iter()
        .map(|refusal| refusal.url.as_str())
        .collect();
    let mut decisive: Option<NetworkError> = None;
    for (index, failures) in &printed.subresources {
        for failed in failures {
            // Reported already, with the remedy, by the refusal line above. The
            // browser reports a refusal as a failed load as well, and it is one
            // thing to say rather than two.
            if refused.contains(failed.url.as_str()) {
                continue;
            }
            if !is_media_file(&failed.url) {
                // The request's own line, as for a document (D48): the URL and
                // Qt's two numbers, under every handler.
                report_failed_request(settings, failed);
                decisive.get_or_insert(failed.error);
                continue;
            }
            // `ignore` is the default, and it used to say nothing at all. A
            // document that renders without its stylesheet is the hardest kind
            // of failure to diagnose precisely because it renders, and
            // wkhtmltopdf named the resource here too (#114).
            report_media(settings, failed);
            if objects[*index].load.on_media_error == LoadErrorHandling::Abort {
                decisive.get_or_insert(failed.error);
            }
        }
    }
    for refusal in &printed.refused {
        decisive.get_or_insert(refusal.error());
    }
    // A document that never arrived is exit 1 under every handler too, and only
    // `abort` kept the file from being written (D44).
    if let Some(failed) = printed.failed.first() {
        decisive.get_or_insert(failed.error);
    }

    let mut outcome = ExitCode::Success;
    if let Some(error) = decisive {
        // wkhtmltopdf's exact wording, and not prefixed with the program name:
        // applications grep for this line. Written once, naming one error, as
        // wkhtmltopdf does.
        println_stderr(&format!(
            "Exit with code 1 due to network error: {}",
            error.name()
        ));
        outcome = ExitCode::Failure;
    }

    say(settings, "Done");
    Ok(outcome)
}

/// Load a band document and measure it (D39).
///
/// On a page of its own, with the file rule the sheets will apply, so what
/// is measured is what will be framed: an image the rule refuses is missing
/// from both.
async fn measure(
    browser: &Browser,
    url: &str,
    prepare: &[plan::Command],
    load: &rchtmltopdf_core::settings::LoadSettings,
    rules: intercept::Rules,
    progress: &Progress,
) -> Result<Measured, ConvertError> {
    let page = browser.new_page().await?;
    let _policing = intercept::install(page.session(), rules).await?;
    page.prepare(prepare).await?;
    let report = page.load(url, load, progress).await?;
    if let Some(failed) = &report.document {
        return Err(rchtmltopdf_browser::Error::Navigation {
            url: failed.url.clone(),
            reason: format!("{} (a header or footer document)", failed.error.name()),
        }
        .into());
    }
    Ok(page.measure_band().await?)
}

/// How much a band document adds to the print margin on its side, in inches:
/// its height when it sizes the margin, nothing when it is fitted into one.
///
/// Times the zoom, because that is the height that lands on the paper (D61).
/// The measurement was taken in a viewport the zoom widened, so it is in that
/// layout's millimetres and not the paper's.
fn reserved_inches(band: &Band, named: bool, measured: &Measured, zoom: f64) -> f64 {
    match plan::sized_by_its_document(band, named) {
        true => measured.height_mm * zoom / 25.4,
        false => 0.0,
    }
}

/// The progress line for one document, in wkhtmltopdf's shape.
///
/// One document keeps the line wrappers have seen since V0. Several say which
/// one this is, because ten identical lines say nothing.
fn loading_line(index: usize, total: usize) -> String {
    if total == 1 {
        "Loading page (1/2)".to_string()
    } else {
        format!("Loading page {} of {total} (1/2)", index + 1)
    }
}

/// A progress line, in wkhtmltopdf's shape and on its stream.
fn say(settings: &Settings, line: &str) {
    if settings.global.log_level.shows_progress() {
        eprintln!("{line}");
    }
}

/// One failed request, in the shape wkhtmltopdf wrote for it.
///
/// `Failed to load <url>, with network status code <n> and http status code
/// <n> - <Name>`. The numbers are Qt's, which is what an application parsing
/// this line expects; the tail is the error's name rather than Qt's sentence
/// about it, because that sentence is Qt's and not reproducible (D48).
fn report_failed_request(settings: &Settings, failed: &Failed) {
    if settings.global.log_level.shows_errors() {
        eprintln!(
            "{PROGRAM}: Failed to load {}, with network status code {} and http status code {} - {}",
            failed.url,
            failed.error.code(),
            failed.http_status,
            failed.error.name()
        );
    }
}

/// One failed media file, named the way wkhtmltopdf names it.
fn report_media(settings: &Settings, failed: &Failed) {
    if settings.global.log_level.shows_warnings() {
        eprintln!("{PROGRAM}: warning: {failed} was not loaded");
    }
}

/// Straight to stderr whatever the level, because this line is a contract.
fn println_stderr(line: &str) {
    eprintln!("{line}");
}

/// The documents to convert, in order.
///
/// Pages and covers. A cover is a page that was given no bands and is left
/// out of the outline and the count; nothing about printing it differs. A
/// table of contents is refused by name: converting the pages around it and saying
/// nothing would produce a document that looks right and is missing part of
/// itself, which is the worst outcome available.
fn pages(settings: &Settings) -> Result<Vec<&ObjectSettings>, ConvertError> {
    if settings.objects.is_empty() {
        return Err(ConvertError::Unsupported("no document to convert".into()));
    }

    // Standard input is a stream, and the second read of it gets nothing.
    // Converting an empty document in its place would look like a page that
    // rendered blank, so it is refused instead.
    let from_stdin = settings
        .objects
        .iter()
        .filter(|object| object.input == Some(Input::Stdin))
        .count();
    if from_stdin > 1 {
        return Err(ConvertError::Unsupported(format!(
            "standard input can be read once, and it was given as {from_stdin} documents"
        )));
    }

    Ok(settings.objects.iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rchtmltopdf_core::settings::ObjectSettings;

    fn with(objects: Vec<ObjectSettings>) -> Settings {
        Settings {
            objects,
            ..Settings::default()
        }
    }

    fn page() -> ObjectSettings {
        ObjectSettings::page(Input::classify("a.html"))
    }

    #[test]
    fn one_page_or_several_are_what_gets_converted() {
        assert_eq!(pages(&with(vec![page()])).unwrap().len(), 1);
        assert_eq!(pages(&with(vec![page(), page(), page()])).unwrap().len(), 3);
    }

    fn heading(
        title: &str,
        page: usize,
        top: f64,
        children: Vec<rchtmltopdf_pdf::OutlineItem>,
    ) -> rchtmltopdf_pdf::OutlineItem {
        rchtmltopdf_pdf::OutlineItem {
            title: title.into(),
            page,
            left: 34.0,
            top,
            children,
        }
    }

    fn measured(title: &str, height: f64) -> HeadingBox {
        HeadingBox {
            title: title.into(),
            height,
        }
    }

    /// The boxes pair with the outline by title, in reading order, and only
    /// on the document's own pages (D57).
    #[test]
    fn back_links_pair_headings_with_their_measured_boxes() {
        let items = [
            heading("Before", 1, 800.0, vec![]),
            heading("One", 2, 803.0, vec![heading("One A", 2, 700.0, vec![])]),
            heading("Two", 3, 790.0, vec![]),
            heading("After", 4, 800.0, vec![]),
        ];
        let boxes = [
            measured("One", 40.0),
            measured("One  A", 30.0),
            measured("Two", 40.0),
        ];
        let links = back_links_of(&items, 2..=3, &boxes, 0.75, 561.0);
        let summary: Vec<(usize, f64, f64)> =
            links.iter().map(|l| (l.page, l.top, l.height)).collect();
        assert_eq!(
            summary,
            [(2, 803.0, 30.0), (2, 700.0, 22.5), (3, 790.0, 30.0)]
        );
        assert!(links.iter().all(|l| l.right == 561.0 && l.left == 34.0));
    }

    /// A heading the measurement did not see gets nothing, and does not take
    /// the next heading's box.
    #[test]
    fn a_heading_without_a_box_gets_no_back_link() {
        let items = [
            heading("One", 1, 803.0, vec![]),
            heading("Hidden", 1, 750.0, vec![]),
            heading("Two", 1, 700.0, vec![]),
        ];
        let boxes = [measured("One", 40.0), measured("Two", 20.0)];
        let links = back_links_of(&items, 1..=1, &boxes, 1.0, 500.0);
        let summary: Vec<(f64, f64)> = links.iter().map(|l| (l.top, l.height)).collect();
        assert_eq!(summary, [(803.0, 40.0), (700.0, 20.0)]);
    }

    #[test]
    fn a_cover_is_converted_like_a_page() {
        let mut cover = page();
        cover.kind = ObjectKind::Cover;
        assert_eq!(pages(&with(vec![cover, page()])).unwrap().len(), 2);
    }

    /// A table of contents is an object like the others, and the only one
    /// with no document to read: it is generated from the pages around it
    /// (D41).
    #[test]
    fn a_table_of_contents_is_an_object_like_the_others() {
        let mut toc = page();
        toc.kind = ObjectKind::Toc;
        toc.input = None;
        assert_eq!(pages(&with(vec![toc, page()])).unwrap().len(), 2);
    }

    /// On its own too: `rchtmltopdf toc out.pdf` is a table of contents of
    /// nothing, and wkhtmltopdf prints the page rather than refusing.
    #[test]
    fn a_table_of_contents_alone_is_still_a_conversion() {
        let mut toc = page();
        toc.kind = ObjectKind::Toc;
        toc.input = None;
        assert_eq!(pages(&with(vec![toc])).unwrap().len(), 1);
    }

    /// The second read of a stream gets nothing, and a blank page in its place
    /// would look like a rendering problem.
    #[test]
    fn standard_input_twice_is_refused_rather_than_read_empty() {
        let stdin = || ObjectSettings::page(Input::Stdin);
        assert!(pages(&with(vec![stdin(), page()])).is_ok());

        let error = pages(&with(vec![stdin(), page(), stdin()])).unwrap_err();
        assert!(error.to_string().contains("standard input"), "{error}");
        assert!(error.to_string().contains('2'), "{error}");
    }

    #[test]
    fn nothing_to_convert_is_reported_rather_than_panicking() {
        assert!(pages(&with(vec![])).is_err());
    }

    /// One document keeps the line wrappers have seen since V0.
    #[test]
    fn the_progress_line_says_which_document_only_when_there_are_several() {
        assert_eq!(loading_line(0, 1), "Loading page (1/2)");
        assert_eq!(loading_line(0, 3), "Loading page 1 of 3 (1/2)");
        assert_eq!(loading_line(2, 3), "Loading page 3 of 3 (1/2)");
    }

    #[test]
    fn nothing_left_names_every_document_that_was_skipped() {
        let error = ConvertError::NothingLeft(vec![
            "could not load a: ContentNotFoundError".into(),
            "could not load b: HostNotFoundError".into(),
        ]);
        let message = error.to_string();
        assert!(message.contains("skip"), "{message}");
        assert!(message.contains("could not load a"), "{message}");
        assert!(message.contains("could not load b"), "{message}");
    }
}
