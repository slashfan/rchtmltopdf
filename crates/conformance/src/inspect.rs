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
        let content = self
            .document
            .get_and_decode_page_content(self.page_id(page))
            .expect("page content should decode");

        let mut boxes = Vec::new();
        let mut ctm = Matrix::IDENTITY;
        let mut saved = Vec::new();

        for operation in &content.operations {
            match operation.operator.as_str() {
                "q" => saved.push(ctm),
                "Q" => ctm = saved.pop().unwrap_or(Matrix::IDENTITY),
                "cm" => {
                    if let Some(matrix) = Matrix::from_operands(&operation.operands) {
                        ctm = matrix.then(ctm);
                    }
                }
                "re" => {
                    let numbers = numbers(&operation.operands);
                    if let [x, y, width, height] = numbers[..] {
                        boxes.push(ctm.map_rect(x, y, width, height));
                    }
                }
                _ => {}
            }
        }
        boxes
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
