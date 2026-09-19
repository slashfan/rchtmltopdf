//! Reading a PDF back, structurally.
//!
//! **The only module in the workspace that knows lopdf exists (D25).** Test
//! bodies see `Rect`, a page count and a `String`; nothing above this line names
//! a lopdf type, so the library can be replaced by editing one file. That is
//! D12's guarantee, kept on the harness side of the fence.
//!
//! Assertions here are structural, never pixel (D15). The project does not
//! promise pixel parity with wkhtmltopdf, and goldens would break on every
//! Chromium and font revision.

use lopdf::{Document, Object, ObjectId};

/// Points per inch. PDF user space is 1/72 inch, and everything here is in it.
pub const POINTS_PER_INCH: f64 = 72.0;

/// How far a measurement may be out and still count as equal.
///
/// **Chromium does not print the paper size it was asked for.** Read from the
/// raw bytes of twenty-four documents — not through this module, to keep lopdf's
/// `f32` out of the evidence — every media box it writes is an exact multiple of
/// 0.24 pt, which is 1/300 inch, and the requested size is moved onto that grid.
///
/// **Which grid point it picks was not worked out**, and the obvious guesses are
/// all wrong: it is not the nearest multiple of 0.24 pt (A4's 595.27 would then
/// print as 595.20, and it prints as 595.92), nor the next one up, and near
/// 8 inches the step is four device units rather than one. What follows is
/// measurement, not a model, and anyone predicting an output from it will be
/// wrong sooner or later.
///
/// What was measured, and what this constant actually rests on:
///
/// - the deviation reaches **0.91 pt** across twenty-four arbitrary widths from
///   3 to 14 inches, and **0.64 pt** across the named sizes asserted here;
/// - it is usually positive but **not always** — 7.777 in comes out 0.024 pt
///   small — so this is a tolerance in both directions, not an allowance for
///   Chromium being generous;
/// - margins do not move it, and it is identical across repeated runs.
///
/// 1.5 pt therefore covers everything observed with about 0.6 pt to spare, while
/// staying far inside the gaps that matter: A4 and Letter, the closest pair
/// asserted here, are 16.7 pt apart in width. It is not a licence to be vague —
/// a size wrong by a whole millimetre, 2.83 pt, still fails.
pub const TOLERANCE: f64 = 1.5;

/// One subpath painted on a page: what it covers, and the colour it carries.
///
/// The bounds are the corners it was built from, mapped through the
/// transformation matrix in force. See [`Pdf::painted_paths`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Subpath {
    pub bounds: Rect,
    pub fill: [f64; 3],
}

impl Subpath {
    /// True when this was filled with the given colour, within the rounding a
    /// colour makes on its way through a content stream.
    pub fn is_about(self, red: f64, green: f64, blue: f64) -> bool {
        let close = |left: f64, right: f64| (left - right).abs() <= 0.01;
        close(self.fill[0], red) && close(self.fill[1], green) && close(self.fill[2], blue)
    }
}

/// A rectangle in PDF user space, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

impl Rect {
    pub fn width(self) -> f64 {
        self.right - self.left
    }

    pub fn height(self) -> f64 {
        self.top - self.bottom
    }

    /// True when this is the given size, in points, within [`TOLERANCE`].
    pub fn is_about(self, width: f64, height: f64) -> bool {
        (self.width() - width).abs() <= TOLERANCE && (self.height() - height).abs() <= TOLERANCE
    }

    fn describe(self) -> String {
        format!(
            "{:.2} x {:.2} pt at ({:.2}, {:.2})",
            self.width(),
            self.height(),
            self.left,
            self.bottom
        )
    }
}

/// A filled rectangle, and what it was filled with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Painted {
    pub rect: Rect,
    /// Red, green and blue, each 0 to 1, as the content stream last set them.
    pub fill: [f64; 3],
}

impl Painted {
    /// Whether this was filled with something close to the given colour.
    ///
    /// Generous on purpose: the question these tests ask is "black or white",
    /// and a thousandth either way is the encoder rounding rather than a
    /// different colour.
    pub fn is_about(self, red: f64, green: f64, blue: f64) -> bool {
        let close = |a: f64, b: f64| (a - b).abs() < 0.01;
        close(self.fill[0], red) && close(self.fill[1], green) && close(self.fill[2], blue)
    }
}

/// The graphics state this walk cares about.
///
/// Both halves are saved and restored by `q` and `Q`. Tracking the matrix and
/// forgetting the colour would attribute one rectangle's fill to another.
#[derive(Debug, Clone, Copy)]
struct State {
    ctm: Matrix,
    fill: [f64; 3],
}

impl Default for State {
    /// A content stream starts in black, per the specification.
    fn default() -> Self {
        Self {
            ctm: Matrix::IDENTITY,
            fill: [0.0, 0.0, 0.0],
        }
    }
}

/// One entry of the outline, as a sidebar lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    /// 1 at the top level, one more for each level down.
    pub level: usize,
    pub title: String,
    /// The 1-based page the entry points at, 0 when it points at nothing.
    pub page: usize,
}

/// A PDF text string as a reader shows it: Latin-1, or UTF-16 big endian when
/// the byte order mark says so.
fn decode_text(bytes: &[u8]) -> String {
    if let Some(units) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = units
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    bytes.iter().map(|byte| *byte as char).collect()
}

/// What a link on a page does, as a reader would follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// To a 1-based page of this file. 0 when the destination names a page
    /// the file does not have, or an anchor the catalog does not know.
    Internal { page: usize },
    /// To a URI.
    External { uri: String },
}

/// A PDF, opened for inspection.
pub struct Pdf {
    document: Document,
}

impl Pdf {
    /// Parse, or fail saying what was actually handed over.
    ///
    /// The failure this catches most often is not a corrupt PDF but a file that
    /// is not one at all: an error page, an empty file, or a message the program
    /// wrote to the wrong stream. So the panic shows the first bytes.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        assert!(
            bytes.starts_with(b"%PDF-"),
            "not a PDF ({} bytes, starting {:?})",
            bytes.len(),
            String::from_utf8_lossy(&bytes[..bytes.len().min(32)])
        );
        let document = Document::load_mem(bytes).expect("a PDF this program wrote should parse");
        Self { document }
    }

    pub fn page_count(&self) -> usize {
        self.document.get_pages().len()
    }

    /// Every font a 1-based page draws with, by the name in the file.
    ///
    /// The name is `BaseFont`, subset prefix and all: Chromium writes
    /// `AAAAAA+NotoSans` for an embedded subset, so a caller asking whether a
    /// face was used looks for a substring rather than for equality. A page
    /// that draws no text has no `/Font` resource and answers nothing.
    pub fn fonts(&self, page: usize) -> Vec<String> {
        let id = self.page_id(page);
        let Some(fonts) = self
            .document
            .get_dictionary(id)
            .ok()
            .and_then(|page| page.get(b"Resources").ok())
            .and_then(|resources| self.document.dereference(resources).ok())
            .and_then(|(_, resources)| resources.as_dict().ok())
            .and_then(|resources| resources.get(b"Font").ok())
            .and_then(|fonts| self.document.dereference(fonts).ok())
            .and_then(|(_, fonts)| fonts.as_dict().ok().cloned())
        else {
            return Vec::new();
        };
        let mut names: Vec<String> = fonts
            .iter()
            .filter_map(|(_, font)| self.document.dereference(font).ok())
            .filter_map(|(_, font)| font.as_dict().ok().cloned())
            .filter_map(|font| {
                let name = font.get(b"BaseFont").and_then(Object::as_name).ok()?;
                Some(String::from_utf8_lossy(name).into_owned())
            })
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// The paper, for a 1-based page number.
    ///
    /// **`MediaBox` may not be on the page.** It is an inheritable attribute, so
    /// a page can leave it to the `Pages` node above it. Chromium writes it per
    /// page today, and reading only the page dictionary would work right up to
    /// the point where V2 merges documents and something normalises the tree.
    /// Walking `Parent` costs four lines and removes the trap.
    pub fn media_box(&self, page: usize) -> Rect {
        let id = self.page_id(page);
        let mut node = id;
        loop {
            let dictionary = self
                .document
                .get_dictionary(node)
                .expect("a page node should be a dictionary");
            if let Ok(value) = dictionary.get(b"MediaBox") {
                return self.rect_from(value);
            }
            match dictionary.get(b"Parent").and_then(Object::as_reference) {
                Ok(parent) => node = parent,
                Err(_) => panic!("page {page} has no MediaBox, and neither has any parent"),
            }
        }
    }

    /// The size of every stream carrying no filter, largest first.
    ///
    /// The browser deflates everything it hands over, so what comes back plain
    /// is what this program wrote itself (D67). The two-byte `q` and `Q` that
    /// wrap a page's own drawing are the ones worth leaving alone.
    pub fn plain_streams(&self) -> Vec<usize> {
        let mut sizes: Vec<usize> = self
            .document
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .filter(|stream| stream.dict.get(b"Filter").is_err())
            .map(|stream| stream.content.len())
            .collect();
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        sizes
    }

    /// Whether the file carries the accessibility structure tree.
    ///
    /// Read from the catalog, which is where a reader looks for it: a file
    /// whose `StructTreeRoot` is gone is untagged whatever its pages still
    /// carry. Chromium writes one unless it is told not to, and wkhtmltopdf
    /// never wrote any (D66).
    pub fn is_tagged(&self) -> bool {
        self.document
            .catalog()
            .is_ok_and(|catalog| catalog.get(b"StructTreeRoot").is_ok())
    }

    /// One entry from the document's Info dictionary, the way a reader shows it.
    ///
    /// Decodes the two string encodings a PDF has: Latin-1, or UTF-16 big endian
    /// when the byte order mark says so. A title with an accent in it is written
    /// the second way, and reading it the first way would pass a test while
    /// showing mojibake to everybody else.
    pub fn info(&self, key: &str) -> Option<String> {
        let entry = self.document.trailer.get(b"Info").ok()?;
        let dictionary = match entry {
            Object::Reference(id) => self.document.get_dictionary(*id).ok()?,
            Object::Dictionary(dictionary) => dictionary,
            _ => return None,
        };

        let bytes = dictionary.get(key.as_bytes()).ok()?.as_str().ok()?;
        Some(decode_text(bytes))
    }

    /// The outline — a reader's bookmarks — flattened in the order a sidebar
    /// lists it, each entry with its depth and the 1-based page it points at.
    ///
    /// Read by walking `First` and `Next` from the catalog's `Outlines`, which
    /// is what a reader does. A level is one deeper than its parent; the top
    /// level is 1.
    pub fn outline(&self) -> Vec<Bookmark> {
        let mut out = Vec::new();
        let Some(root) = self
            .document
            .catalog()
            .ok()
            .and_then(|catalog| catalog.get(b"Outlines").ok())
            .and_then(|entry| entry.as_reference().ok())
        else {
            return out;
        };
        let by_id: std::collections::BTreeMap<ObjectId, usize> = self
            .document
            .get_pages()
            .into_iter()
            .map(|(number, id)| (id, number as usize))
            .collect();
        self.walk_outline(root, 1, &by_id, &mut out);
        out
    }

    /// The box of every link annotation on a 1-based page, in the order the
    /// page lists them. Points from the bottom-left, whichever way the
    /// corners were written.
    pub fn link_rects(&self, page: usize) -> Vec<Rect> {
        let id = self.page_id(page);
        let Some(annotations) = self
            .document
            .get_dictionary(id)
            .ok()
            .and_then(|page| page.get(b"Annots").ok())
            .and_then(|annots| self.document.dereference(annots).ok())
            .and_then(|(_, annots)| annots.as_array().ok())
        else {
            return Vec::new();
        };
        annotations
            .iter()
            .filter_map(|annotation| self.document.dereference(annotation).ok())
            .filter_map(|(_, annotation)| annotation.as_dict().ok())
            .filter(|annotation| {
                annotation.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Link")
            })
            .filter_map(|annotation| {
                let rect = annotation.get(b"Rect").ok()?.as_array().ok()?;
                let at = |index: usize| {
                    rect.get(index)
                        .and_then(|value| value.as_float().ok())
                        .map(f64::from)
                        .unwrap_or(0.0)
                };
                Some(Rect {
                    left: at(0).min(at(2)),
                    bottom: at(1).min(at(3)),
                    right: at(0).max(at(2)),
                    top: at(1).max(at(3)),
                })
            })
            .collect()
    }

    /// The links on a 1-based page, in the order the page lists them.
    ///
    /// A named destination is looked up in the catalog's `Dests` dictionary,
    /// which is where Chromium writes `<a href="#x">`; an explicit one and a
    /// `GoTo` action are followed to their page; a `URI` action is read as it
    /// is. Anything else on the page — a widget, a highlight — is not a link
    /// and is left out.
    pub fn links(&self, page: usize) -> Vec<Link> {
        let id = self.page_id(page);
        let by_id: std::collections::BTreeMap<ObjectId, usize> = self
            .document
            .get_pages()
            .into_iter()
            .map(|(number, id)| (id, number as usize))
            .collect();
        let page_of = |destination: &Object| -> usize {
            self.document
                .dereference(destination)
                .ok()
                .and_then(|(_, d)| d.as_array().ok())
                .and_then(|array| array.first())
                .and_then(|first| first.as_reference().ok())
                .and_then(|id| by_id.get(&id).copied())
                .unwrap_or(0)
        };
        let named = |name: &[u8]| -> usize {
            self.document
                .catalog()
                .ok()
                .and_then(|catalog| catalog.get(b"Dests").ok())
                .and_then(|dests| self.document.dereference(dests).ok())
                .and_then(|(_, dests)| dests.as_dict().ok())
                .and_then(|dests| dests.get(name).ok())
                .map(page_of)
                .unwrap_or(0)
        };

        let Some(annotations) = self
            .document
            .get_dictionary(id)
            .ok()
            .and_then(|page| page.get(b"Annots").ok())
            .and_then(|annots| self.document.dereference(annots).ok())
            .and_then(|(_, annots)| annots.as_array().ok())
        else {
            return Vec::new();
        };
        annotations
            .iter()
            .filter_map(|annotation| self.document.dereference(annotation).ok())
            .filter_map(|(_, annotation)| annotation.as_dict().ok())
            .filter(|annotation| {
                annotation.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Link")
            })
            .filter_map(|annotation| {
                if let Ok(destination) = annotation.get(b"Dest") {
                    let page = match destination {
                        Object::Name(name) | Object::String(name, _) => named(name),
                        other => page_of(other),
                    };
                    return Some(Link::Internal { page });
                }
                let (_, action) = self.document.dereference(annotation.get(b"A").ok()?).ok()?;
                let action = action.as_dict().ok()?;
                match action.get(b"S").and_then(Object::as_name).ok()? {
                    b"URI" => Some(Link::External {
                        uri: String::from_utf8_lossy(action.get(b"URI").ok()?.as_str().ok()?)
                            .into_owned(),
                    }),
                    b"GoTo" => Some(Link::Internal {
                        page: action.get(b"D").ok().map(page_of).unwrap_or(0),
                    }),
                    _ => None,
                }
            })
            .collect()
    }

    fn walk_outline(
        &self,
        parent: ObjectId,
        level: usize,
        pages: &std::collections::BTreeMap<ObjectId, usize>,
        out: &mut Vec<Bookmark>,
    ) {
        let mut next = self
            .document
            .get_dictionary(parent)
            .ok()
            .and_then(|d| d.get(b"First").and_then(Object::as_reference).ok());
        let mut seen = 0;
        while let Some(id) = next {
            seen += 1;
            assert!(seen < 10_000, "the outline chain loops");
            let entry = self.document.get_dictionary(id).expect("an outline entry");
            let title = entry
                .get(b"Title")
                .ok()
                .and_then(|title| title.as_str().ok())
                .map(decode_text)
                .unwrap_or_default();
            let page = entry
                .get(b"Dest")
                .ok()
                .and_then(|dest| self.document.dereference(dest).ok())
                .and_then(|(_, dest)| dest.as_array().ok())
                .and_then(|array| array.first())
                .and_then(|page| page.as_reference().ok())
                .and_then(|id| pages.get(&id).copied())
                .unwrap_or(0);
            out.push(Bookmark { level, title, page });
            self.walk_outline(id, level + 1, pages, out);
            next = entry.get(b"Next").and_then(Object::as_reference).ok();
        }
    }

    /// Every character on every page, in reading order.
    ///
    /// Used for a sentinel, not for layout. Extraction inserts its own
    /// whitespace between text-showing operators, so assert that a phrase is
    /// present, never that the whole string equals something.
    pub fn text(&self) -> String {
        let pages: Vec<u32> = self.document.get_pages().keys().copied().collect();
        self.document
            .extract_text(&pages)
            .expect("text should be extractable")
    }

    /// The characters on one 1-based page, with the same caveats as [`text`].
    ///
    /// What a merge is asserted with: that a sentinel is on the page its
    /// document was given at, and on no other.
    ///
    /// [`text`]: Pdf::text
    pub fn page_text(&self, page: usize) -> String {
        let number = u32::try_from(page).expect("a page number");
        assert!(
            self.document.get_pages().contains_key(&number),
            "no page {page} in a {} page document",
            self.page_count()
        );
        self.document
            .extract_text(&[number])
            .expect("text should be extractable")
    }

    /// Every rectangle painted on a 1-based page, in page coordinates.
    ///
    /// This is how a margin gets measured. **A margin has no dictionary entry of
    /// its own** — it is an offset applied to content, and nothing in the file
    /// records it. So a fixture paints a block that fills its content area, and
    /// where that block lands *is* the margin.
    ///
    /// The trap is that `re` operands are in the current transformation matrix,
    /// not in page space, and Chromium emits a `cm` before painting. Reading the
    /// operands raw gives numbers that look plausible and are wrong, so the
    /// matrix stack is tracked and every corner is mapped through it.
    pub fn painted_boxes(&self, page: usize) -> Vec<Rect> {
        self.painted(page)
            .into_iter()
            .map(|fill| fill.rect)
            .collect()
    }

    /// Every filled rectangle, **and the colour it was filled with**.
    ///
    /// The colour is not decoration. `--no-background` does not stop Chromium
    /// emitting a `<div>`'s background rectangle: it emits the same rectangle in
    /// the same place and fills it **white**. A test that measures geometry
    /// cannot tell the two apart, and would pass whether the option worked or
    /// not.
    ///
    /// Only the device colour operators are followed — `g`, `rg` and `k`.
    /// `sc`/`scn` depend on a colour space set elsewhere in the resources, and
    /// nothing Chromium writes for a fixture here uses them; a rectangle filled
    /// through one keeps the last colour seen rather than guessing.
    pub fn painted(&self, page: usize) -> Vec<Painted> {
        let content = self
            .document
            .get_and_decode_page_content(self.page_id(page))
            .expect("page content should decode");

        let mut painted = Vec::new();
        let mut state = State::default();
        let mut saved: Vec<State> = Vec::new();

        for operation in &content.operations {
            let operands = numbers(&operation.operands);
            match operation.operator.as_str() {
                // `q` and `Q` save and restore the colour with the matrix: a
                // rectangle filled inside a saved block does not change what is
                // filled after it.
                "q" => saved.push(state),
                "Q" => state = saved.pop().unwrap_or_default(),
                "cm" => {
                    if let Some(matrix) = Matrix::from_operands(&operation.operands) {
                        state.ctm = matrix.then(state.ctm);
                    }
                }
                "g" => {
                    if let [grey] = operands[..] {
                        state.fill = [grey, grey, grey];
                    }
                }
                "rg" => {
                    if let [red, green, blue] = operands[..] {
                        state.fill = [red, green, blue];
                    }
                }
                "k" => {
                    if let [cyan, magenta, yellow, black] = operands[..] {
                        let channel = |ink: f64| (1.0 - ink) * (1.0 - black);
                        state.fill = [channel(cyan), channel(magenta), channel(yellow)];
                    }
                }
                "re" => {
                    if let [x, y, width, height] = operands[..] {
                        painted.push(Painted {
                            rect: state.ctm.map_rect(x, y, width, height),
                            fill: state.fill,
                        });
                    }
                }
                _ => {}
            }
        }
        painted
    }

    /// Every subpath painted on a 1-based page: where it lies, and the colour
    /// it was filled with.
    ///
    /// **Why this exists next to [`painted`].** That one reads `re`, the
    /// rectangle operator, which is what a background or a block is. A great
    /// deal of what a browser draws is not an `re` at all. A **dashed border**
    /// is the case that forced this: Chromium emits each dash as a four-point
    /// subpath — `m`, then three `l` — and fills the run of them with a single
    /// `f` at the end. A test looking for rectangles found the page background
    /// and nothing else, and a `--disable-dotted-lines` that did nothing would
    /// have passed it.
    ///
    /// Only what is actually painted is reported: a path ended by `n` is a
    /// clipping path, not ink, and every page here opens with one. Curve
    /// operators contribute their endpoint, which is enough to bound a path
    /// and wrong for a curve that bulges outside its endpoints — no test here
    /// measures one.
    ///
    /// [`painted`]: Pdf::painted
    pub fn painted_paths(&self, page: usize) -> Vec<Subpath> {
        let content = self
            .document
            .get_and_decode_page_content(self.page_id(page))
            .expect("page content should decode");

        let mut done: Vec<Subpath> = Vec::new();
        let mut pending: Vec<Vec<(f64, f64)>> = Vec::new();
        let mut state = State::default();
        let mut saved: Vec<State> = Vec::new();

        for operation in &content.operations {
            let operands = numbers(&operation.operands);
            match operation.operator.as_str() {
                "q" => saved.push(state),
                "Q" => state = saved.pop().unwrap_or_default(),
                "cm" => {
                    if let Some(matrix) = Matrix::from_operands(&operation.operands) {
                        state.ctm = matrix.then(state.ctm);
                    }
                }
                "g" => {
                    if let [grey] = operands[..] {
                        state.fill = [grey, grey, grey];
                    }
                }
                "rg" => {
                    if let [red, green, blue] = operands[..] {
                        state.fill = [red, green, blue];
                    }
                }
                "k" => {
                    if let [cyan, magenta, yellow, black] = operands[..] {
                        let channel = |ink: f64| (1.0 - ink) * (1.0 - black);
                        state.fill = [channel(cyan), channel(magenta), channel(yellow)];
                    }
                }
                // A new subpath begins, whatever the last one was doing.
                "m" => {
                    if let [x, y] = operands[..] {
                        pending.push(vec![(x, y)]);
                    }
                }
                // Straight and curved segments alike extend the current one.
                "l" | "c" | "v" | "y" => {
                    if let (Some(current), [.., x, y]) = (pending.last_mut(), &operands[..]) {
                        current.push((*x, *y));
                    }
                }
                // A rectangle is a subpath too, and closed as it stands.
                "re" => {
                    if let [x, y, width, height] = operands[..] {
                        pending.push(vec![
                            (x, y),
                            (x + width, y),
                            (x + width, y + height),
                            (x, y + height),
                        ]);
                    }
                }
                // Painted, one way or another: the path becomes ink.
                "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "S" | "s" => {
                    for points in pending.drain(..) {
                        if let Some(bounds) = state.ctm.map_points(&points) {
                            done.push(Subpath {
                                bounds,
                                fill: state.fill,
                            });
                        }
                    }
                }
                // Not painted: a clipping path, which every page opens with.
                "n" => pending.clear(),
                _ => {}
            }
        }
        done
    }

    /// The largest painted rectangle on a page, which is the one a fixture uses
    /// to mark out its content area.
    pub fn largest_painted_box(&self, page: usize) -> Rect {
        let boxes = self.painted_boxes(page);
        boxes
            .iter()
            .copied()
            .max_by(|a, b| {
                (a.width() * a.height())
                    .partial_cmp(&(b.width() * b.height()))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_else(|| {
                panic!(
                    "nothing was painted on page {page}; the fixture should fill its content area"
                )
            })
    }

    /// A human-readable summary, for when an assertion fails.
    pub fn describe(&self) -> String {
        let pages = self.page_count();
        let sizes: Vec<String> = (1..=pages)
            .map(|page| self.media_box(page).describe())
            .collect();
        format!("{pages} page(s): {}", sizes.join("; "))
    }

    fn page_id(&self, page: usize) -> ObjectId {
        let pages = self.document.get_pages();
        let number = u32::try_from(page).expect("a page number fits in u32");
        *pages
            .get(&number)
            .unwrap_or_else(|| panic!("no page {page}; the document has {}", pages.len()))
    }

    fn rect_from(&self, value: &Object) -> Rect {
        let (_, resolved) = self
            .document
            .dereference(value)
            .expect("a MediaBox should resolve");
        let values = numbers(
            resolved
                .as_array()
                .expect("a MediaBox should be an array of four numbers"),
        );
        let [left, bottom, right, top] = values[..] else {
            panic!("a MediaBox should have four numbers, found {values:?}");
        };
        // The array is not required to be ordered, so normalise rather than
        // trusting it.
        Rect {
            left: left.min(right),
            bottom: bottom.min(top),
            right: left.max(right),
            top: bottom.max(top),
        }
    }
}

fn numbers(objects: &[Object]) -> Vec<f64> {
    objects
        .iter()
        .filter_map(|object| match object {
            Object::Integer(value) => Some(*value as f64),
            Object::Real(value) => Some(f64::from(*value)),
            _ => None,
        })
        .collect()
}

/// A PDF transformation matrix, in the order the specification writes it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Matrix {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Matrix {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    fn from_operands(operands: &[Object]) -> Option<Self> {
        let values = numbers(operands);
        let [a, b, c, d, e, f] = values[..] else {
            return None;
        };
        Some(Self { a, b, c, d, e, f })
    }

    /// This transformation followed by `outer`, which is what `cm` does to the
    /// current matrix.
    fn then(self, outer: Self) -> Self {
        Self {
            a: self.a * outer.a + self.b * outer.c,
            b: self.a * outer.b + self.b * outer.d,
            c: self.c * outer.a + self.d * outer.c,
            d: self.c * outer.b + self.d * outer.d,
            e: self.e * outer.a + self.f * outer.c + outer.e,
            f: self.e * outer.b + self.f * outer.d + outer.f,
        }
    }

    fn point(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// The box a run of points covers, every one of them mapped first.
    ///
    /// `None` for a subpath with no points, which a stream can produce with a
    /// paint operator and nothing to paint.
    fn map_points(self, points: &[(f64, f64)]) -> Option<Rect> {
        if points.is_empty() {
            return None;
        }
        let mapped: Vec<(f64, f64)> = points.iter().map(|(x, y)| self.point(*x, *y)).collect();
        let xs = mapped.iter().map(|(x, _)| *x);
        let ys = mapped.iter().map(|(_, y)| *y);
        Some(Rect {
            left: xs.clone().fold(f64::INFINITY, f64::min),
            bottom: ys.clone().fold(f64::INFINITY, f64::min),
            right: xs.fold(f64::NEG_INFINITY, f64::max),
            top: ys.fold(f64::NEG_INFINITY, f64::max),
        })
    }

    /// Map all four corners, not two. A matrix may flip or rotate, and taking
    /// only the opposite corners would produce a negative width that silently
    /// compares equal to nothing.
    fn map_rect(self, x: f64, y: f64, width: f64, height: f64) -> Rect {
        let corners = [
            self.point(x, y),
            self.point(x + width, y),
            self.point(x, y + height),
            self.point(x + width, y + height),
        ];
        let xs: Vec<f64> = corners.iter().map(|(x, _)| *x).collect();
        let ys: Vec<f64> = corners.iter().map(|(_, y)| *y).collect();
        Rect {
            left: xs.iter().copied().fold(f64::INFINITY, f64::min),
            bottom: ys.iter().copied().fold(f64::INFINITY, f64::min),
            right: xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            top: ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: f64, bottom: f64, right: f64, top: f64) -> Rect {
        Rect {
            left,
            bottom,
            right,
            top,
        }
    }

    #[test]
    fn a_size_comparison_allows_half_a_point_and_no_more() {
        let a4 = rect(0.0, 0.0, 595.28, 841.89);
        assert!(a4.is_about(595.28, 841.89));
        assert!(
            a4.is_about(595.92, 841.92),
            "Chromium's actual A4 should match"
        );
        // A5 is half of A4. Nothing this loose should ever confuse the two.
        assert!(!a4.is_about(419.53, 595.28));
        // A millimetre out is 2.83 pt, and must still fail.
        assert!(!a4.is_about(595.28 + 2.83, 841.89));
    }

    // --- the transformation matrix -------------------------------------------

    #[test]
    fn an_untransformed_rectangle_keeps_its_coordinates() {
        assert_eq!(
            Matrix::IDENTITY.map_rect(10.0, 20.0, 100.0, 200.0),
            rect(10.0, 20.0, 110.0, 220.0)
        );
    }

    /// The trap this exists for: Chromium emits `cm` before painting, so raw
    /// `re` operands are not page coordinates.
    #[test]
    fn a_translation_moves_the_rectangle() {
        let shifted = Matrix {
            e: 50.0,
            f: 70.0,
            ..Matrix::IDENTITY
        };
        assert_eq!(
            shifted.map_rect(0.0, 0.0, 100.0, 100.0),
            rect(50.0, 70.0, 150.0, 170.0)
        );
    }

    #[test]
    fn a_scale_resizes_it() {
        let doubled = Matrix {
            a: 2.0,
            d: 3.0,
            ..Matrix::IDENTITY
        };
        assert_eq!(
            doubled.map_rect(10.0, 10.0, 5.0, 5.0),
            rect(20.0, 30.0, 30.0, 45.0)
        );
    }

    /// A flip is the case that makes mapping two corners wrong: it produces a
    /// rectangle with negative extent, which then matches no assertion at all.
    #[test]
    fn a_flip_still_produces_a_positive_rectangle() {
        let flipped = Matrix {
            d: -1.0,
            f: 800.0,
            ..Matrix::IDENTITY
        };
        let mapped = flipped.map_rect(10.0, 10.0, 100.0, 50.0);
        assert_eq!(mapped, rect(10.0, 740.0, 110.0, 790.0));
        assert!(mapped.width() > 0.0 && mapped.height() > 0.0);
    }

    /// `cm` concatenates onto the current matrix rather than replacing it, so a
    /// scale inside a translation has to compose in that order.
    #[test]
    fn transformations_compose_in_the_order_cm_applies_them() {
        let scale = Matrix {
            a: 2.0,
            d: 2.0,
            ..Matrix::IDENTITY
        };
        let translate = Matrix {
            e: 100.0,
            f: 0.0,
            ..Matrix::IDENTITY
        };
        // Scale first, then translate: a unit square at the origin becomes 2x2
        // at x = 100.
        assert_eq!(
            scale.then(translate).map_rect(0.0, 0.0, 1.0, 1.0),
            rect(100.0, 0.0, 102.0, 2.0)
        );
        // The other order scales the translation too.
        assert_eq!(
            translate.then(scale).map_rect(0.0, 0.0, 1.0, 1.0),
            rect(200.0, 0.0, 202.0, 2.0)
        );
    }

    #[test]
    fn a_matrix_needs_exactly_six_numbers() {
        assert!(Matrix::from_operands(&[Object::Integer(1), Object::Integer(0)]).is_none());
        let six: Vec<Object> = (0..6).map(|_| Object::Integer(1)).collect();
        assert!(Matrix::from_operands(&six).is_some());
    }
}
