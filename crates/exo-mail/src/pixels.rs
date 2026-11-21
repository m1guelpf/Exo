//! Read-receipt pixels, which the pipeline strips. `Show all images` loads them on their own
//! ([`crate::load_read_receipts`]).

use std::sync::LazyLock;

use crate::{
    dom::{self, NodeRef, Style},
    sanitize::decode_uri_component,
};

/// URL fragments of known open-tracking services.
const TRACKERS: &[&str] = &[
    "r.superhuman.com",
    "r.staging-superhuman.com",
    "http://mkt4477.com/open",
    "airbnb.com/tracking/pixel",
    "track.affinity.co",
    "bl-1.com",
    "mailstat.us/tr",
    ".net/mail_track",
    ".net/on.jsp",
    "contactmonkey.com/api/v1/tracker",
    "tracking.cirrusinsight.com",
    "close.io/email_opened",
    "r20.rs6.net/on.jsp",
    "constantcontact.com/images/p1x1.gif",
    "prosperworks.com/tp/t",
    "customeriomail.com/e/o",
    "track.customer.io/e/o",
    "trackedlink.net",
    "dripemail2",
    "exct.net/open.aspx",
    "web.frontapp.com/api",
    "email81.com",
    "google-analytics.com/collect",
    "t.hubspotemail.net/e2t/o",
    "t.hsms06.com",
    "click.icptrack.com/icp",
    "infusionsoft.com/app/emailOpened",
    "via.intercom.io/o",
    "via.intercom-mail-",
    "ec2-52-26-194-35.us-west-2.compute.amazonaws.com/x",
    "www.google.com/appserve/mkt/img/",
    "launchbit.com/taz-pixel",
    "emltrk.com",
    "list-manage.com/track",
    "/track/open.php",
    "mailbutler.io/tracking",
    "email.mailgun.(.*)/o/",
    "mltrk.io/pixel/",
    "mailtrack.io",
    "mandrillapp.com/track",
    "list-manage.com/track/open",
    "list-manage1.com/track/open",
    "ping.answerbook.com",
    "mkto-",
    "/trk",
    "azure.microsoft.com/emailtrackingpixel",
    "email.mixmax.com/e/o",
    "track.mixmax.com/api",
    "app.mixmax.com/api/track",
    "mixpanel.com/track",
    "nethunt.com/api/v1/track/email/",
    "app.outreach.io",
    "outrch.com/api/mailings/opened",
    "/ut.php",
    "polymail.io",
    "pstmrk.it/open",
    "cmail1.com/t",
    "app.relateiq.com/t.png",
    "go.rjmetrics.com",
    "returnpath.net/pixel.gif",
    "track.getrocket.com/",
    "salesforceiq.com/t.png",
    "salesloftlinks.com/email_trackers",
    "sdr.salesloft.com/email_trackers",
    "/wf/open",
    "track.getsidekick.com",
    "t.senal",
    "t.signaux",
    "t.signale",
    "t.sigopn",
    "t.sidekickopen",
    "t.strk",
    "mailfoogae.appspot.com",
    "strongview.com/t",
    "go.toutapp.com",
    "tinyletterapp.com",
    "trackmyemails.org",
    "t.xink.io/Tracking",
    "yamm-track.appspot",
    "app.yesware.com",
    "t.yesware.com",
    "pixel.wp.com/t.gif",
    "ec2-52-26-194-35",
];

/// What a removed pixel's `src` becomes: a transparent 1×1 GIF.
pub(crate) const BLANK_GIF: &str =
    "data:image/gif;base64,R0lGODlhAQABAPAAAAAAAAAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";

/// Whether an `<img>` is a read receipt: a known tracker, or at most 4×4 px.
pub(crate) fn is_read_status_pixel(image: &NodeRef) -> bool {
    let src = decode_uri_component(&dom::attr(image, "src").unwrap_or_default());
    if TRACKERS.iter().any(|tracker| src.contains(tracker)) {
        return true;
    }
    let style = Style::of(image);
    let (Some(width), Some(height)) = (
        rendered_size(image, &style, "width"),
        rendered_size(image, &style, "height"),
    ) else {
        return false;
    };
    width <= 4 && height <= 4
}

/// Whether an `<img>` with `attributes` is a read receipt, as [`is_read_status_pixel`] judges
/// one in a message: for images outside a message's body (a draft's).
pub fn is_read_receipt(attributes: &[(&str, &str)]) -> bool {
    is_read_status_pixel(&dom::element("img", attributes))
}

/// `min(max(size, min-size), max-size)` from inline px values and the attribute; `None` when
/// unknown or in other units.
fn rendered_size(image: &NodeRef, style: &Style, axis: &str) -> Option<i64> {
    static INTEGER: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"^\s*[-+]?\d+").expect("valid regex"));
    let number = |text: &str| {
        INTEGER
            .find(text)
            .and_then(|m| m.as_str().trim().parse::<i64>().ok())
    };
    // `Err` is a length in a unit other than px.
    let px = |name: &str| -> Result<Option<i64>, ()> {
        match style.get(name) {
            "" => Ok(None),
            value => match value.strip_suffix("px") {
                Some(px) => Ok(number(px)),
                None => Err(()),
            },
        }
    };
    let size = match px(axis).ok()? {
        Some(size) => Some(size),
        None => dom::attr(image, axis).and_then(|value| number(&value)),
    };
    let min = px(&format!("min-{axis}")).ok()?;
    let max = px(&format!("max-{axis}")).ok()?;
    // lodash's `max`/`min` skip missing values and give `undefined` for none.
    let low = [size, min].into_iter().flatten().max();
    [low, max].into_iter().flatten().min()
}

/// Neutralizes every read-receipt pixel under `root` and returns their URLs.
pub(crate) fn remove_read_status_pixels(root: &NodeRef) -> Vec<String> {
    let mut urls = Vec::new();
    for image in dom::select(root, |n| dom::is(n, "img")) {
        let Some(src) = dom::attr(&image, "src").filter(|src| !src.is_empty()) else {
            continue;
        };
        if !is_read_status_pixel(&image) {
            continue;
        }
        urls.push(src);
        dom::set_attr(&image, "src", BLANK_GIF);
        dom::remove_attr(&image, "srcset");
        dom::remove_attr(&image, "sizes");
        for axis in ["width", "height"] {
            if dom::attr(&image, axis).is_none_or(|value| value.is_empty()) {
                dom::set_attr(&image, axis, "1");
            }
        }
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(html: &str) -> bool {
        let document = dom::parse(html);
        is_read_status_pixel(&dom::select_first(&document, |n| dom::is(n, "img")).unwrap())
    }

    #[test]
    fn spots_trackers_and_tiny_images() {
        assert!(pixel("<img src=\"https://t.yesware.com/t/x.gif\">"));
        // The tracker is in an encoded parameter after an `&`.
        assert!(pixel(
            "<img src=\"https://x.io/r?a=1&amp;u=https%3A%2F%2Fmailtrack.io%2Fp.gif\">"
        ));
        assert!(pixel(
            "<img src=\"https://x.io/o.gif\" width=\"1\" height=\"1\">"
        ));
        assert!(pixel(
            "<img src=\"https://x.io/o.gif\" style=\"width:0px;height:0px\">"
        ));
        assert!(!pixel(
            "<img src=\"https://x.io/logo.png\" width=\"120\" height=\"40\">"
        ));
        assert!(!pixel("<img src=\"https://x.io/logo.png\">"));
        assert!(!pixel(
            "<img src=\"https://x.io/o.gif\" width=\"1\" style=\"height:1em\">"
        ));
    }
}
