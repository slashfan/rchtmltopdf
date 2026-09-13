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

use crate::{PROGRAM, VERSION, input, numbering, output, toc};
use rchtmltopdf_browser::Browser;
use rchtmltopdf_browser::LaunchOptions;
use rchtmltopdf_browser::band::{self, Edge, Measured, Sheet};
use rchtmltopdf_browser::clock;
use rchtmltopdf_browser::deadline;
use rchtmltopdf_browser::intercept;
use rchtmltopdf_browser::locate::{SystemEnvironment, locate};
use rchtmltopdf_browser::placeholder;
use rchtmltopdf_browser::plan::{self, Plan};
use rchtmltopdf_browser::render::{Failed, Progress};
use rchtmltopdf_core::settings::{Band, ObjectKind, ObjectSettings, Settings};
use rchtmltopdf_core::{ExitCode, Input, LoadErrorHandling};
use std::collections::HashMap;
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
            ConvertError::Input(error) => write!(f, "{error}"),
            ConvertError::Output(error) => write!(f, "{error}"),
            ConvertError::Browser(error) => write!(f, "{error}"),
            ConvertError::Pdf(error) => write!(f, "{error}"),
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

/// What came out of the browser for the documents that made it.
struct Printed {
    /// One PDF per document that was printed, in command line order, with the
    /// index of the object it came from.
    documents: Vec<(usize, Vec<u8>)>,
    /// Subresources the interception refused, across every document.
    refused: Vec<intercept::Refusal>,
    /// Subresources that failed, paired with the object whose
    /// `--load-media-error-handling` judges them.
    media: Vec<(usize, Vec<Failed>)>,
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
    printed: &'a [(usize, Vec<u8>)],
    tables: &'a [(usize, Vec<u8>)],
) -> Vec<(usize, &'a [u8])> {
    let mut all: Vec<(usize, &[u8])> = printed
        .iter()
        .map(|(index, pdf)| (*index, pdf.as_slice()))
        .chain(tables.iter().map(|(index, pdf)| (*index, pdf.as_slice())))
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
    printed: &'a [(usize, Vec<u8>)],
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
) -> Result<Vec<(usize, Vec<u8>)>, ConvertError> {
    // Where each printed document started in the merge the headings were read
    // off, which is what carries a heading from that merge to the finished file.
    let mut merged_first: HashMap<usize, usize> = HashMap::new();
    let mut running = 1usize;
    for ((index, _), pages) in job.printed.iter().zip(job.counts) {
        merged_first.insert(*index, running);
        running += pages;
    }

    let mut lengths: Vec<usize> = vec![1; job.tables.len()];
    let mut printed: Vec<(usize, Vec<u8>)> = Vec::new();
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
        let shares: Vec<numbering::Part> = pieces
            .iter()
            .map(|(index, pages)| numbering::Part {
                pages: *pages,
                page_offset: job.plans[*index].finish.numbering.page_offset,
            })
            .collect();
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
                |page| numbering::dump_page(&shares, page),
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
            printed.push((*index, page.print_to_pdf(&job.plans[*index].print).await?));
        }

        let measured: Vec<usize> = printed
            .iter()
            .map(|(_, pdf)| {
                rchtmltopdf_pdf::merge(&[rchtmltopdf_pdf::Part {
                    pdf,
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
    printed: &[(usize, Vec<u8>)],
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
                input::resolve(source)?
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
    let (pdf, printed) = deadline::within(settings.global.timeout, &progress, async {
        let mut printed = Printed {
            documents: Vec::with_capacity(total),
            refused: Vec::new(),
            media: Vec::new(),
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
                reserved_inches(&object.header, page_setup.named.top, &heights[index].header),
                reserved_inches(
                    &object.footer,
                    page_setup.named.bottom,
                    &heights[index].footer,
                ),
            );

            say(settings, &loading_line(index, total));
            let page = browser.new_page().await?;

            // Before anything is fetched, the document included: the policy has
            // to be in place for the first request, not the second (D10).
            let policing = intercept::install(page.session(), plan.requests.clone()).await?;

            page.prepare(&plan.prepare).await?;
            let report = page.load(document.url(), &object.load, &progress).await?;

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
                match object.load.on_document_error {
                    LoadErrorHandling::Abort => {
                        return Err(rchtmltopdf_browser::Error::Navigation {
                            url: failed.url.clone(),
                            reason: failed.error.name().to_string(),
                        }
                        .into());
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

            printed
                .documents
                .push((index, page.print_to_pdf(&print).await?));

            // Read before the guard is dropped, which is what stops interception.
            printed.refused.extend(
                policing
                    .as_ref()
                    .map(intercept::Interception::refused)
                    .unwrap_or_default(),
            );
            printed.media.push((index, report.media));
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
            .map(|(index, pdf)| rchtmltopdf_pdf::Part {
                pdf,
                url: &plans[*index].finish.document_url,
                links: &plans[*index].finish.links,
                contents: false,
            })
            .collect();

        let tables_printed: Vec<(usize, Vec<u8>)> = if tables.is_empty() {
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
            // The headings to list. Generated whatever `--no-outline` says:
            // that option is about the bookmarks the file carries, and a table
            // of contents was asked for separately.
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
            .map(|(index, pdf)| rchtmltopdf_pdf::Part {
                pdf,
                url: &plans[*index].finish.document_url,
                links: &plans[*index].finish.links,
                // The file is named after the first document, and a table of
                // contents is not one: wkhtmltopdf passed over its own.
                contents: tables.contains(index),
            })
            .collect();
        let merged = rchtmltopdf_pdf::merge(&parts)?;

        // How each document's pages count, in the order they were merged. The
        // dump reads it for `--page-offset` (D40) and the bands for `[page]`.
        let shares: Vec<numbering::Part> = ordered
            .iter()
            .zip(&merged.pages)
            .map(|((index, _), pages)| numbering::Part {
                pages: *pages,
                page_offset: plans[*index].finish.numbering.page_offset,
            })
            .collect();

        // The outline the browser wrote is all or nothing per document, so the
        // depth is cut here, and the dump describes what the file will carry.
        // The treatment is global, so the first plan's copy is every plan's.
        let finish = &plans[0].finish;
        let (pdf, items) = rchtmltopdf_pdf::outline(
            &merged.pdf,
            &rchtmltopdf_pdf::OutlineTreatment {
                keep: finish.outline.enabled,
                depth: finish.outline.depth,
            },
        )?;
        if let Some(path) = &finish.outline.dump {
            let xml = crate::outline::xml(&items, |page| numbering::dump_page(&shares, page));
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
            let numbered = numbering::number(&shares, &items);

            // `[title]` is the document's own `<title>` and `[doctitle]` the
            // finished file's, and neither was known when the plan was made
            // (#110). Chromium wrote each document's title into the part it
            // printed, so they are read back off the parts: the file's is
            // `--title`, or the first part that is not a table of contents —
            // the same part the merge took its Info dictionary from.
            let titles: Vec<String> = ordered
                .iter()
                .map(|(_, pdf)| Ok(rchtmltopdf_pdf::title(pdf)?.unwrap_or_default()))
                .collect::<Result<_, ConvertError>>()?;
            let file_title = match &settings.global.title {
                Some(given) => given.clone(),
                None => ordered
                    .iter()
                    .zip(&titles)
                    .find(|((index, _), _)| !tables.contains(index))
                    .map(|(_, title)| title.clone())
                    .unwrap_or_default(),
            };
            let contexts: Vec<placeholder::Context> = ordered
                .iter()
                .zip(&titles)
                .map(|((index, _), document_title)| placeholder::Context {
                    title: file_title.clone(),
                    document_title: document_title.clone(),
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
        if let Some(browser) = browser {
            browser.close().await?;
        }
        Ok((pdf, printed))
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
    // file afterwards. The same pass names the producer and dates the document
    // (#29).
    let pdf = rchtmltopdf_pdf::set_metadata(
        &pdf,
        &rchtmltopdf_pdf::Metadata {
            title: settings.global.title.clone(),
            producer: format!("{PROGRAM} {VERSION}"),
            creator: format!("{PROGRAM} {VERSION}"),
            created: now,
        },
    )?;

    // Written before the media errors are judged, because D14 says a subresource
    // that failed under `abort` produces the document *and* exits 1. A wrapper
    // that raises on the exit code still has the PDF to look at.
    output::write(&settings.global.output, &pdf)?;

    // Each document is judged by its own handler. The first `abort` decides the
    // exit code and writes the line applications grep for; the rest are only
    // reported.
    let mut outcome = ExitCode::Success;
    for (index, failures) in &printed.media {
        match (objects[*index].load.on_media_error, failures.as_slice()) {
            (_, []) | (LoadErrorHandling::Ignore, _) => {}
            (LoadErrorHandling::Skip, failures) => {
                for failed in failures {
                    report_media(settings, failed);
                }
            }
            (LoadErrorHandling::Abort, failures) => {
                for failed in failures {
                    report_media(settings, failed);
                }
                if outcome == ExitCode::Success {
                    // wkhtmltopdf's exact wording, and not prefixed with the
                    // program name: applications grep for this line.
                    println_stderr(&format!(
                        "Exit with code 1 due to network error: {}",
                        failures[0].error.name()
                    ));
                }
                outcome = ExitCode::Failure;
            }
        }
    }

    // A document that never arrived is exit 1 under every handler, and only
    // `abort` kept the file from being written (D44). Said once: a media
    // failure under `abort` has already written the same line, and wkhtmltopdf
    // prints it once too, naming one error.
    if let Some(failed) = printed.failed.first()
        && outcome == ExitCode::Success
    {
        println_stderr(&format!(
            "Exit with code 1 due to network error: {}",
            failed.error.name()
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
fn reserved_inches(band: &Band, named: bool, measured: &Measured) -> f64 {
    match plan::sized_by_its_document(band, named) {
        true => measured.height_mm / 25.4,
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

/// One failed subresource, named the way wkhtmltopdf names it.
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
