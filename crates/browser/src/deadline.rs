//! Bounding a whole conversion.
//!
//! wkhtmltopdf has no deadline at all, which leaves a browser that never settles
//! blocking its caller indefinitely. A web worker rendering an invoice cannot
//! afford that, so D16 puts one limit over the entire run: finding a browser,
//! starting it, loading, waiting and printing.
//!
//! It sits here rather than inside the waiting, on purpose. Several of those
//! steps are unbounded by design, because a page that long-polls never settles
//! and no individual step can tell the difference between slow and never. One
//! limit above all of them can.
//!
//! Expiry produces no document. D16 is explicit that a silent partial PDF in an
//! invoicing pipeline is worse than a failure, because nothing downstream can
//! tell it apart from a whole one.

use crate::error::Error;
use crate::render::Progress;
use std::future::Future;
use std::time::Duration;

/// Run something under the conversion's deadline.
///
/// `None` means no limit, which is what `--timeout 0` asks for and what
/// wkhtmltopdf always did.
///
/// The failure names the rung the page was on when time ran out. "Timed out"
/// alone tells nobody whether to raise the limit, fix the document, or go and
/// look at the network.
///
/// Generic over the error, so a caller whose work can fail in ways of its own
/// — a merge, a file that would not be written — can put all of it under the
/// one limit. The timeout arrives as this crate's error, converted.
pub async fn within<F, T, E>(
    limit: Option<Duration>,
    progress: &Progress,
    work: F,
) -> std::result::Result<T, E>
where
    F: Future<Output = std::result::Result<T, E>>,
    E: From<Error>,
{
    let Some(limit) = limit else {
        return work.await;
    };

    match tokio::time::timeout(limit, work).await {
        Ok(outcome) => outcome,
        Err(_) => Err(E::from(Error::Timeout {
            after: limit,
            stage: progress.current().describe(),
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Stage;

    #[tokio::test]
    async fn work_that_finishes_in_time_returns_its_answer() {
        let progress = Progress::new();
        let answer = within(Some(Duration::from_secs(5)), &progress, async {
            Ok::<_, Error>(42)
        })
        .await
        .unwrap();
        assert_eq!(answer, 42);
    }

    #[tokio::test]
    async fn a_failure_of_its_own_is_passed_through_unchanged() {
        let progress = Progress::new();
        let outcome: Result<(), Error> = within(Some(Duration::from_secs(5)), &progress, async {
            Err(Error::ConnectionClosed)
        })
        .await;
        assert!(matches!(outcome, Err(Error::ConnectionClosed)));
    }

    /// The whole point: the failure says what it interrupted.
    #[tokio::test]
    async fn expiry_names_the_rung_the_page_was_on() {
        let progress = Progress::new();
        let watched = progress.clone();

        let outcome: Result<(), Error> =
            within(Some(Duration::from_millis(50)), &progress, async move {
                watched.enter_for_test(Stage::AwaitingNetworkIdle);
                // Never finishes, which is exactly the case a deadline exists for.
                std::future::pending::<()>().await;
                Ok(())
            })
            .await;

        match outcome {
            Err(error @ Error::Timeout { .. }) => {
                assert_eq!(
                    error.to_string(),
                    "timed out after 0s while waiting for the network to go idle"
                );
            }
            other => panic!("expected a timeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_rung_reported_is_the_one_current_at_expiry() {
        let progress = Progress::new();
        let watched = progress.clone();

        let outcome: Result<(), Error> =
            within(Some(Duration::from_millis(80)), &progress, async move {
                watched.enter_for_test(Stage::AwaitingLoad);
                tokio::time::sleep(Duration::from_millis(20)).await;
                watched.enter_for_test(Stage::AwaitingFonts);
                std::future::pending::<()>().await;
                Ok(())
            })
            .await;

        assert!(
            outcome.unwrap_err().to_string().contains("web fonts"),
            "should report where it had got to, not where it started"
        );
    }

    /// `--timeout 0` asks for no limit, which is what wkhtmltopdf always gave.
    #[tokio::test]
    async fn no_limit_means_no_limit() {
        let progress = Progress::new();
        let answer = within(None, &progress, async {
            // Longer than any limit a test would set, and it still completes.
            tokio::time::sleep(Duration::from_millis(120)).await;
            Ok::<_, Error>("finished")
        })
        .await
        .unwrap();
        assert_eq!(answer, "finished");
    }
}
