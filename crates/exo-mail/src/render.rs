//! A message's render: its body after the pipeline, ready to show and to cache.

/// One message's sanitized, linkified, quote-split and classified body. It is plain data, so the
/// store can cache it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Render {
    /// The whole cleaned message as a document. The next message in the thread finds its quote
    /// of this one by comparing against it.
    pub html: String,
    /// The new part of the message, as HTML to place inside the body container.
    pub unquoted_html: String,
    /// What hides behind the `…`: quoted earlier messages, and signatures.
    pub quoted_html: String,
    /// Whether there is a quoted part (and so a `…`).
    pub has_quoted_content: bool,
    /// The sender's `<body>` attributes, which move onto the body container.
    pub body_attributes: BodyAttributes,
    /// Whether the new part looks like marketing mail, which keeps its own design.
    pub unquoted_is_marketing: bool,
    /// Whether the quoted part looks like marketing mail.
    pub quoted_is_marketing: bool,
    /// The text a reader sees, without the quoted part (for search and copy).
    pub visible_text: String,
    /// The first 200 characters of `visible_text`.
    pub snippet: String,
    /// Read-receipt pixels that were removed; non-empty means "Email tracking blocked" unless
    /// the Images setting shows all images, which loads them ([`crate::load_read_receipts`]).
    pub read_status_urls: Vec<String>,
    /// Something in the body loads from the network, which blocking remote content stops
    /// ⌘K Show Images is offered for it.
    pub has_remote_images: bool,
    /// Every link, in document order (for "Open Link", ⌘O).
    pub links: Vec<Link>,
    /// The link the Unsubscribe action opens when the headers have none.
    pub unsubscribe_link: Option<Link>,
    /// One-click actions, in document order.
    pub actions: Vec<crate::Action>,
    /// What a Google Calendar notification says about its event.
    pub calendar_data: crate::CalendarData,
}

/// The `<body>` attributes the shown body carries over.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BodyAttributes {
    /// `class`.
    pub class: Option<String>,
    /// `id`.
    pub id: Option<String>,
    /// `style`, with `bgcolor` folded in as `background-color` when the style has none.
    pub style: Option<String>,
    /// Whether the body sets a background color.
    pub has_background: bool,
}

/// A link in a message body.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    /// What the link says (its text, or the `alt` of an image inside it).
    pub text: String,
    /// The sentence around it.
    pub context: String,
    /// Where it goes.
    pub url: String,
}
