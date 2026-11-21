//! Machine-sent mail: newsletters, promotions, notifications, receipts, social, forums and
//! spam, each with the rich HTML its (invented) sender would send.

use gmail::mime::{Attachment, Mailbox};
use jiff::{ToSpan, Zoned};

use super::compose::{List, Post, State, Thread, escape};
use super::rng::Rng;
use super::{assets, cast};

/// A bulk sender's identity and look.
struct Brand {
    name: &'static str,
    email: &'static str,
    /// Header bar color.
    color: &'static str,
    /// Postal address for the footer (CAN-SPAM).
    address: &'static str,
    category: &'static str,
}

const PRODUCT_BRIEF: Brand = Brand {
    name: "The Product Brief",
    email: "hello@productbrief.news",
    color: "#1f2937",
    address: "The Product Brief, 2180 Alder St, Oakland, CA 94612",
    category: "CATEGORY_UPDATES",
};
const MORNING_LEDGER: Brand = Brand {
    name: "Morning Ledger",
    email: "daily@morningledger.news",
    color: "#0f766e",
    address: "Morning Ledger Media, 61 Lispenard Row, New York, NY 10013",
    category: "CATEGORY_UPDATES",
};
const FREIGHT_SIGNALS: Brand = Brand {
    name: "Freight Signals",
    email: "newsletter@freightsignals.com",
    color: "#b45309",
    address: "Freight Signals, 1 Industrial Way, Columbus, OH 43215",
    category: "CATEGORY_UPDATES",
};
const FIELD_NOTES: Brand = Brand {
    name: "Ana Ruiz",
    email: "ana@letters.anaruiz.com",
    color: "#7c3aed",
    address: "Field Notes by Ana Ruiz, PO Box 2210, Portland, OR 97208",
    category: "CATEGORY_UPDATES",
};
const KETTLE: Brand = Brand {
    name: "Kettle & Crumb",
    email: "hello@kettleandcrumb.com",
    color: "#7c2d12",
    address: "Kettle & Crumb Coffee Co., 19 Front St, Burlington, VT 05401",
    category: "CATEGORY_PROMOTIONS",
};
const FIELD_FELL: Brand = Brand {
    name: "Field & Fell Outfitters",
    email: "deals@fieldandfell.com",
    color: "#166534",
    address: "Field & Fell Outfitters, 300 Summit Ave, Boulder, CO 80302",
    category: "CATEGORY_PROMOTIONS",
};
const LINEN_HOUSE: Brand = Brand {
    name: "Linen House",
    email: "news@linenhouse.co",
    color: "#a16207",
    address: "Linen House, 12 Mercer St, New York, NY 10013",
    category: "CATEGORY_PROMOTIONS",
};
const NORTHSTAR: Brand = Brand {
    name: "Northstar Air",
    email: "offers@northstarair.com",
    color: "#1d4ed8",
    address: "Northstar Air, 2500 Airport Blvd, Denver, CO 80249",
    category: "CATEGORY_PROMOTIONS",
};

const fn notifier(
    name: &'static str,
    email: &'static str,
    color: &'static str,
    category: &'static str,
) -> Brand {
    Brand {
        name,
        email,
        color,
        address: "",
        category,
    }
}

const TRACKLIGHT: Brand = notifier(
    "Tracklight",
    "notifications@tracklight.dev",
    "#2563eb",
    "CATEGORY_UPDATES",
);
const QUORUM: Brand = notifier(
    "Quorum",
    "digest@quorum.chat",
    "#4f46e5",
    "CATEGORY_UPDATES",
);
const DOCKET: Brand = notifier(
    "Docket",
    "comments@docket.so",
    "#0ea5e9",
    "CATEGORY_UPDATES",
);
const PARCELPOINT: Brand = notifier(
    "Parcelpoint",
    "updates@parcelpoint.com",
    "#ea580c",
    "CATEGORY_UPDATES",
);
const LEDGERLY: Brand = notifier(
    "Ledgerly",
    "receipts@ledgerly.app",
    "#059669",
    "CATEGORY_UPDATES",
);
const HARBOR_BANK: Brand = notifier(
    "Harbor Bank",
    "alerts@harborbank.com",
    "#0f172a",
    "CATEGORY_UPDATES",
);
const RESERVATIONS: Brand = notifier(
    "Northstar Air",
    "reservations@northstarair.com",
    "#1d4ed8",
    "CATEGORY_UPDATES",
);
const LINKWISE: Brand = notifier(
    "Linkwise",
    "notifications@linkwise.com",
    "#0a66c2",
    "CATEGORY_SOCIAL",
);
const PRODUCTOPS: Brand = notifier(
    "ProductOps Community",
    "digest@community.productops.org",
    "#9333ea",
    "CATEGORY_FORUMS",
);

/// What a bulk message says: subject, body paragraphs and one call to action.
struct Content {
    subject: String,
    paragraphs: Vec<String>,
    cta: (&'static str, String),
}

/// Every machine-sent thread, ending at `now`.
pub fn threads(now: &Zoned, rng: &mut Rng) -> Vec<Thread> {
    let mut threads = Vec::new();
    newsletters(now, rng, &mut threads);
    promotions(now, rng, &mut threads);
    notifications(now, rng, &mut threads);
    social_and_forums(now, rng, &mut threads);
    spam(now, rng, &mut threads);
    threads
}

/// A local time `days` before `now`'s date, at `hour:minute`.
fn at(now: &Zoned, days: i64, hour: i8, minute: i8) -> Zoned {
    let date = now.date() - days.days();
    date.at(hour, minute, 0, 0)
        .to_zoned(now.time_zone().clone())
        .expect("fixture times exist in the account zone")
}

/// A one-message bulk thread. Recent mail sits in the inbox (mostly unread, the newest
/// always); older mail was archived.
fn deliver(
    brand: &Brand,
    when: Zoned,
    now: &Zoned,
    subject: String,
    (text, html): (String, String),
    inbox_days: i64,
) -> Thread {
    let age_days = (now.date() - when.date()).get_days();
    let recent = i64::from(age_days) < inbox_days;
    // Deterministic per message: anything from today is unread, older ones about half.
    let unread = recent && (age_days == 0 || super::compose::fnv(&subject) % 2 == 0);
    Thread {
        subject,
        posts: vec![Post {
            from: Mailbox::new(brand.name, brand.email),
            to: vec![cast::me().mailbox()],
            cc: Vec::new(),
            text,
            html: Some(html),
            calendar: None,
            attachments: Vec::new(),
            at: when.timestamp(),
        }],
        state: State {
            inbox: recent,
            unread: usize::from(unread),
            category: Some(brand.category),
            ..State::default()
        },
        list: Some(List {
            id: format!("{} <{}>", brand.name, brand.email.replace('@', ".list.")),
            unsubscribe: format!(
                "https://{}/unsubscribe?u=maya%40brightmoor.co",
                brand
                    .email
                    .split_once('@')
                    .map_or("example.com", |(_, d)| d)
            ),
        }),
        quote: false,
    }
}

/// Email-safe newsletter HTML: a centered 600px table with inline styles.
fn newsletter(brand: &Brand, headline: &str, content: &Content) -> String {
    let preheader = content.paragraphs.first().map_or("", String::as_str);
    let body: String = content
        .paragraphs
        .iter()
        .map(|p| {
            format!(
                "<p style=\"margin:0 0 16px;font-size:16px;line-height:26px;color:#374151\">{}</p>",
                escape(p)
            )
        })
        .collect();
    format!(
        r#"<!DOCTYPE html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title></head>
<body style="margin:0;padding:0;background:#f3f4f6;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif">
<div style="display:none;max-height:0;overflow:hidden;opacity:0">{preheader}</div>
<table role="presentation" width="100%" cellpadding="0" cellspacing="0" style="background:#f3f4f6"><tr><td align="center" style="padding:24px 12px">
<table role="presentation" width="600" cellpadding="0" cellspacing="0" style="max-width:600px;width:100%;background:#ffffff;border-radius:8px;overflow:hidden">
<tr><td style="background:{color};padding:20px 32px;color:#ffffff;font-size:20px;font-weight:700;letter-spacing:0.2px">{name}</td></tr>
<tr><td style="padding:32px 32px 8px"><h1 style="margin:0 0 20px;font-size:26px;line-height:34px;color:#111827">{headline}</h1>{body}</td></tr>
<tr><td style="padding:0 32px 32px"><a href="{href}" style="display:inline-block;background:{color};color:#ffffff;text-decoration:none;font-weight:600;padding:12px 22px;border-radius:6px">{label}</a></td></tr>
<tr><td style="padding:20px 32px;background:#f9fafb;color:#6b7280;font-size:12px;line-height:18px">You're receiving this because you subscribed at {domain}.<br>{address}<br><a href="https://{domain}/unsubscribe" style="color:#6b7280">Unsubscribe</a> · <a href="https://{domain}/preferences" style="color:#6b7280">Email preferences</a></td></tr>
</table></td></tr></table></body></html>"#,
        title = escape(headline),
        preheader = escape(preheader),
        color = brand.color,
        name = escape(brand.name),
        headline = escape(headline),
        href = content.cta.1,
        label = escape(content.cta.0),
        domain = brand.email.split_once('@').map_or("", |(_, d)| d),
        address = escape(brand.address),
    )
}

fn plain(brand: &Brand, headline: &str, content: &Content) -> String {
    format!(
        "{headline}\n\n{}\n\n{}: {}\n\n--\n{}\nUnsubscribe: https://{}/unsubscribe",
        content.paragraphs.join("\n\n"),
        content.cta.0,
        content.cta.1,
        brand.address,
        brand.email.split_once('@').map_or("", |(_, d)| d)
    )
}

/// A newsletter or promotion issue.
fn issue(
    brand: &Brand,
    when: Zoned,
    now: &Zoned,
    headline: &str,
    content: Content,
    inbox_days: i64,
) -> Thread {
    let body = (
        plain(brand, headline, &content),
        newsletter(brand, headline, &content),
    );
    deliver(brand, when, now, content.subject, body, inbox_days)
}

fn newsletters(now: &Zoned, rng: &mut Rng, out: &mut Vec<Thread>) {
    const BRIEFS: &[(&str, &str)] = &[
        (
            "Why onboarding checklists stopped working",
            "Checklists promise momentum, but most users abandon them by step three. We looked at what replaced them in a dozen B2B products.",
        ),
        (
            "The pricing page teardown: nine B2B tools",
            "Annual toggles, highlighted tiers, hidden comparison tables: what actually moves conversion, and what just looks tidy.",
        ),
        (
            "What we learned from 40 churn interviews",
            "Customers rarely leave over a missing feature. They leave when the product stops fitting how their team works.",
        ),
        (
            "Roadmaps are a communication tool, not a plan",
            "The best roadmaps we've seen are short, honest about uncertainty, and rewritten every quarter.",
        ),
        (
            "The case for fewer settings",
            "Every setting is a decision you pushed onto your users. Here's how three teams cut theirs in half.",
        ),
        (
            "How ops software wins the second year",
            "Year one is about the champion. Year two is about the people who never asked for the tool.",
        ),
        (
            "Shipping on Fridays, revisited",
            "The rule was never about Fridays. It was about whether you can roll back without a hero.",
        ),
        (
            "Writing specs people actually read",
            "Lead with the decision, keep the background short, and put open questions where reviewers will see them.",
        ),
        (
            "Activation metrics that predict retention",
            "The best predictor in our dataset wasn't a feature. It was a second person from the same company logging in.",
        ),
        (
            "When to say no to your biggest customer",
            "A framework for custom requests that keeps the roadmap yours.",
        ),
        (
            "Designing empty states that teach",
            "An empty table is the most-viewed screen in your product. Treat it like onboarding.",
        ),
        (
            "A field guide to usage-based pricing",
            "Metering, invoicing, and the conversations sales has to learn to have.",
        ),
        (
            "The quiet art of release notes",
            "Release notes are marketing your existing users actually read. Write them like it.",
        ),
    ];
    for (week, (headline, lede)) in BRIEFS.iter().enumerate() {
        // Thursdays at 7:00.
        let days = i64::try_from(week).unwrap_or(0) * 7
            + (i64::from(now.weekday().to_monday_one_offset()) + 3) % 7;
        let when = at(now, days, 7, 0);
        let paragraphs = vec![
            (*lede).to_owned(),
            "This week we talked to product leaders at four companies about what they changed, what they measured, and what they'd do differently. A few patterns stood out, and one surprised us.".to_owned(),
            "Also in this issue: a reader question about running betas with design partners, and three links worth your time.".to_owned(),
        ];
        let content = Content {
            subject: (*headline).to_owned(),
            paragraphs,
            cta: (
                "Read the full issue",
                "https://productbrief.news/issues/latest".into(),
            ),
        };
        out.push(issue(&PRODUCT_BRIEF, when, now, headline, content, 3));
    }

    const LEADS: &[&str] = &[
        "Rates hold steady as freight volumes tick up",
        "Retail inventories are back to pre-2020 levels",
        "The holiday shipping crunch starts early this year",
        "Why warehouse rents finally stopped climbing",
        "Consumer spending cools, but not where you'd expect",
        "Small-business hiring beats forecasts again",
        "What a weaker dollar means for importers",
        "The quiet boom in regional airlines",
        "Diesel prices fall for a third straight week",
        "Mortgage applications rise on lower rates",
        "Why every grocery chain is building micro-fulfillment",
        "Venture funding for logistics startups rebounds",
        "The case for and against a December rate cut",
        "Manufacturing surveys point to a soft landing",
        "Port congestion eases ahead of peak season",
        "Ad spending shifts again as streaming grows",
        "Why corporate travel is still below 2019",
        "Holiday hiring: fewer seasonal jobs, higher pay",
        "Electric truck orders double year over year",
        "What the latest jobs report actually says",
    ];
    for days in 0..90 {
        let when = at(now, days, 6, 30);
        if when.weekday().to_monday_one_offset() > 5 || when > *now {
            continue;
        }
        let lead = *rng.pick(LEADS);
        let subject = format!("{}: {lead}", when.strftime("%A, %b %-d"));
        let paragraphs = vec![
            format!("Good morning. {lead}, and markets are taking it in stride. Here's what you need to know before your first meeting."),
            "Markets: stocks finished mixed yesterday while bond yields edged lower. Oil slipped for the second day in a row.".to_owned(),
            "Worth your time: a long read on how a family-owned trucking company tripled in size by doing almost everything the opposite of its competitors.".to_owned(),
        ];
        let content = Content {
            subject,
            paragraphs,
            cta: (
                "Read today's edition",
                "https://morningledger.news/today".into(),
            ),
        };
        out.push(issue(&MORNING_LEDGER, when, now, lead, content, 2));
    }

    const SIGNALS: &[&str] = &[
        "Spot rates rise 4% as peak season arrives",
        "Cross-docking makes a comeback",
        "Driver retention: what's working in 2026",
        "The real cost of empty miles",
        "Shippers are rewriting their RFP playbooks",
        "How mid-size 3PLs are using routing software",
        "Detention time is the new battleground",
        "Intermodal volumes climb for a fifth month",
        "Inside the race to digitize the bill of lading",
        "What carriers want from shippers in 2027",
        "Cold-chain capacity is tighter than it looks",
        "The freight recession is over. Now what?",
    ];
    for (week, headline) in SIGNALS.iter().enumerate() {
        let days = i64::try_from(week).unwrap_or(0) * 7
            + (i64::from(now.weekday().to_monday_one_offset()) + 6) % 7;
        let when = at(now, days, 8, 15);
        let paragraphs = vec![
            format!("{headline}. We spoke with dispatch managers, brokers and carriers about what changed this quarter."),
            "Plus: the five lanes with the biggest rate swings this week, and a new survey on how shippers choose software.".to_owned(),
        ];
        let content = Content {
            subject: format!("Freight Signals: {headline}"),
            paragraphs,
            cta: (
                "Read on freightsignals.com",
                "https://freightsignals.com/latest".into(),
            ),
        };
        let mut thread = issue(&FREIGHT_SIGNALS, when, now, headline, content, 4);
        thread.state.labels.push("Newsletters");
        out.push(thread);
    }

    const ESSAYS: &[(&str, &str)] = &[
        (
            "On designing for people who work in a hurry",
            "Dispatchers, nurses, line cooks: the best software for people under time pressure gets out of the way fast.",
        ),
        (
            "Tables are underrated",
            "We keep replacing tables with cards, and our users keep exporting to spreadsheets. Maybe they're telling us something.",
        ),
        (
            "The quiet power of defaults",
            "Most of your users will never change a setting. That makes every default a design decision.",
        ),
        (
            "Color as information",
            "A field guide to using color for meaning, not decoration, and why gray is your most important color.",
        ),
        (
            "Keyboard shortcuts are a love letter",
            "Power users notice when you care about speed. Here's how to design shortcuts that feel inevitable.",
        ),
    ];
    for (index, (headline, lede)) in ESSAYS.iter().enumerate() {
        let when = at(now, i64::try_from(index).unwrap_or(0) * 14 + 2, 12, 0);
        let paragraphs = vec![
            (*lede).to_owned(),
            "I've been thinking about this since a conversation with a reader last month, who described her team's tools as \"fine, until it's busy\". That's the whole essay, really. But let me make the case properly.".to_owned(),
            "As always, reply to this email if you have thoughts. I read every response.".to_owned(),
            "Ana".to_owned(),
        ];
        let content = Content {
            subject: (*headline).to_owned(),
            paragraphs,
            cta: (
                "Read on the web",
                "https://letters.anaruiz.com/latest".into(),
            ),
        };
        out.push(issue(&FIELD_NOTES, when, now, headline, content, 3));
    }
}

fn promotions(now: &Zoned, rng: &mut Rng, out: &mut Vec<Thread>) {
    let promos: &[(&Brand, &[(&str, &str)])] = &[
        (
            &KETTLE,
            &[
                (
                    "☕ 20% off the Holiday Blend, this week only",
                    "Our Holiday Blend is back: dark chocolate, orange peel and a little spice. Use code COZY20 through Sunday.",
                ),
                (
                    "Your next bag ships Thursday",
                    "Your subscription bag of Ethiopia Guji ships Thursday. Want to swap it or skip a week? You can change it until Wednesday at noon.",
                ),
                (
                    "New: a washed Colombia from Huila",
                    "Bright, juicy, and our roaster's favorite of the season. Limited to 400 bags.",
                ),
                (
                    "The gift guide for coffee people",
                    "Grinders, mugs, and subscriptions for everyone on your list, starting at $18.",
                ),
                (
                    "Last chance: free shipping ends tonight",
                    "Free shipping on every order ends at midnight. No code needed.",
                ),
                (
                    "Brew guide: the perfect French press",
                    "Coarse grind, 4 minutes, and the one step almost everyone skips.",
                ),
            ],
        ),
        (
            &FIELD_FELL,
            &[
                (
                    "Early Black Friday: 30% off insulated jackets 🧥",
                    "Members get early access to our biggest sale of the year. The Ridgeline Down Jacket is 30% off through Monday.",
                ),
                (
                    "The 2026 winter boot guide",
                    "We tested 14 boots in slush, ice and city salt. Here are the five we'd buy.",
                ),
                (
                    "Members: double points this weekend",
                    "Earn 2x points on everything through Sunday. Points never expire.",
                ),
                (
                    "Trail-tested: the new Summit 28 pack",
                    "28 liters, a back panel that actually breathes, and pockets where your hands expect them.",
                ),
                (
                    "Your cart misses you",
                    "The Merino Base Layer in Slate (M) is still in your cart, and it's 15% off today.",
                ),
            ],
        ),
        (
            &LINEN_HOUSE,
            &[
                (
                    "The Winter Collection is here",
                    "Heavyweight flax, new colors in clay and fog, and our softest duvet yet.",
                ),
                (
                    "Up to 40% off bedding",
                    "Our annual bedding event is on. Sheets, duvets and quilts, while they last.",
                ),
                (
                    "Made to last: how we wash our linen",
                    "Stone-washed for softness, pre-shrunk so it fits the same in year five.",
                ),
            ],
        ),
        (
            &NORTHSTAR,
            &[
                (
                    "Fares from $89 to Chicago this winter ✈️",
                    "Book by Friday for travel January through March. Nonstop from LaGuardia and Newark.",
                ),
                (
                    "Your miles are about to grow",
                    "Earn 3x miles on every flight booked through December 31.",
                ),
            ],
        ),
    ];
    for (brand, offers) in promos {
        for (index, (subject, lede)) in offers.iter().enumerate() {
            let days = i64::try_from(index).unwrap_or(0) * rng.between(4, 9) + rng.between(0, 3);
            let when = at(
                now,
                days,
                i8::try_from(rng.between(9, 16)).unwrap_or(10),
                i8::try_from(rng.between(0, 59)).unwrap_or(0),
            );
            if when > *now {
                continue;
            }
            let paragraphs = vec![
                (*lede).to_owned(),
                "Questions? Our team is here to help seven days a week. Just reply to this email."
                    .to_owned(),
            ];
            let shop = format!(
                "https://{}/shop",
                brand.email.split_once('@').map_or("", |(_, d)| d)
            );
            let content = Content {
                subject: (*subject).to_owned(),
                paragraphs,
                cta: ("Shop now", shop),
            };
            let mut thread = issue(brand, when, now, subject, content, 2);
            // Linen House mail older than a week was deleted.
            if brand.email == LINEN_HOUSE.email && days > 7 {
                thread.state.trash = true;
                thread.state.inbox = false;
            }
            out.push(thread);
        }
    }
}

/// A notification email: short, branded, one call to action, no mailing-list headers.
fn notice(brand: &Brand, when: Zoned, now: &Zoned, content: Content, inbox_days: i64) -> Thread {
    let lines: String = content
        .paragraphs
        .iter()
        .map(|line| {
            format!(
                "<p style=\"margin:0 0 12px;font-size:14px;line-height:21px;color:#24292f\">{}</p>",
                escape(line)
            )
        })
        .collect();
    let html = format!(
        r#"<!DOCTYPE html><html><body style="margin:0;padding:24px;background:#ffffff;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif">
<table role="presentation" width="100%" style="max-width:560px"><tr><td style="border-top:4px solid {color};padding-top:16px">{lines}
<p style="margin:20px 0"><a href="{href}" style="background:{color};color:#ffffff;padding:8px 16px;border-radius:6px;text-decoration:none;font-size:14px;font-weight:600">{label}</a></p>
<p style="margin:24px 0 0;font-size:12px;color:#6e7781">You are receiving this because of your notification settings. <a href="{href}/settings" style="color:#6e7781">Manage notifications</a></p>
</td></tr></table></body></html>"#,
        color = brand.color,
        href = content.cta.1,
        label = escape(content.cta.0),
    );
    let text = format!(
        "{}\n\n{}: {}",
        content.paragraphs.join("\n\n"),
        content.cta.0,
        content.cta.1
    );
    let mut thread = deliver(brand, when, now, content.subject, (text, html), inbox_days);
    thread.list = None;
    thread
}

fn notifications(now: &Zoned, rng: &mut Rng, out: &mut Vec<Thread>) {
    const PULLS: &[(&str, u32, &str, &str)] = &[
        (
            "atlas",
            1423,
            "Fix timezone conversion for API-created appointments",
            "hiro",
        ),
        (
            "atlas",
            1418,
            "Circuit breaker for the carrier availability API",
            "oliver",
        ),
        ("atlas", 1415, "Jitter the availability cache TTL", "hiro"),
        (
            "atlas",
            1409,
            "Rate-card import behind a feature flag",
            "priya",
        ),
        (
            "atlas",
            1402,
            "Show hours-of-service warnings before confirmation",
            "oliver",
        ),
        (
            "atlas",
            1397,
            "Add cross-dock transfer model (draft)",
            "priya",
        ),
        (
            "atlas",
            1391,
            "Speed up route scoring by caching lane stats",
            "hiro",
        ),
        ("web", 882, "Pricing page v2", "tom"),
        ("web", 879, "Launch page behind password", "lena"),
        ("web", 871, "Customer story: Calloway Freight", "lena"),
        ("onboarding", 217, "Carrier groups step", "tom"),
        ("onboarding", 214, "Skip link contrast fix", "tom"),
        (
            "onboarding",
            209,
            "Track onboarding completion events",
            "ines",
        ),
    ];
    const EVENTS: &[(&str, &str)] = &[
        ("requested your review on", "Review requested"),
        ("approved", "Approved"),
        ("merged", "Merged"),
        ("commented on", "New comment"),
    ];
    for (index, (repo, number, title, author)) in PULLS.iter().enumerate() {
        let author = cast::get(author);
        let events = 2 + rng.below(3);
        for event in 0..events {
            let (verb, _) =
                EVENTS[usize::try_from((event + rng.below(2)) % EVENTS.len() as u64).unwrap_or(0)];
            let days = i64::try_from(index).unwrap_or(0) * 2 + i64::try_from(event).unwrap_or(0);
            let when = at(
                now,
                days,
                i8::try_from(rng.between(9, 18)).unwrap_or(12),
                i8::try_from(rng.between(0, 59)).unwrap_or(0),
            );
            if when > *now {
                continue;
            }
            let content = Content {
                subject: format!("[brightmoor/{repo}] {title} (PR #{number})"),
                paragraphs: vec![
                    format!("{} {verb} this pull request.", author.name()),
                    format!("#{number} {title}"),
                    format!(
                        "brightmoor/{repo} · {} changed files · +{} −{}",
                        rng.between(1, 14),
                        rng.between(10, 420),
                        rng.between(0, 180)
                    ),
                ],
                cta: (
                    "View pull request",
                    format!("https://tracklight.dev/brightmoor/{repo}/pull/{number}"),
                ),
            };
            out.push(notice(&TRACKLIGHT, when, now, content, 1));
        }
    }
    for (days, status, commit) in [
        (0, "passed", "9b1e044"),
        (1, "failed", "3f2a9c1"),
        (4, "failed", "a17c3d2"),
        (8, "passed", "5e90b1f"),
    ] {
        let when = at(now, days, 6, 12);
        if when > *now {
            continue;
        }
        let failed = status == "failed";
        let content = Content {
            subject: format!("[brightmoor/atlas] Build {status}: main ({commit})"),
            paragraphs: vec![
                format!("Build {status} on main ({commit})."),
                if failed {
                    "1 failing job: integration-tests (route_scoring::lane_cache_eviction)"
                } else {
                    "All 214 checks passed in 6m 41s."
                }
                .to_owned(),
            ],
            cta: (
                "View build",
                "https://tracklight.dev/brightmoor/atlas/builds".into(),
            ),
        };
        let brand = Brand {
            color: if failed { "#dc2626" } else { "#16a34a" },
            ..TRACKLIGHT
        };
        let mut thread = notice(&brand, when, now, content, 1);
        thread.state.important = failed;
        out.push(thread);
    }

    const CHANNELS: &[(&str, &str)] = &[
        ("#atlas-launch", "lena"),
        ("#leadership", "daniel"),
        ("#design", "tom"),
        ("#support", "chloe"),
        ("#eng", "priya"),
    ];
    for days in (0..60).step_by(3).chain([1, 4, 7]) {
        let (channel, who) = *rng.pick(CHANNELS);
        let who = cast::get(who);
        let count = rng.between(2, 14);
        let when = at(now, days, 7, i8::try_from(rng.between(0, 40)).unwrap_or(0));
        if when > *now {
            continue;
        }
        let content = Content {
            subject: format!("{count} new messages in {channel}"),
            paragraphs: vec![
                format!("You have {count} unread messages in {channel}."),
                format!(
                    "{}: \"Can someone take a look at the latest draft before EOD? Mostly wording.\"",
                    who.name()
                ),
            ],
            cta: ("Open Quorum", "https://brightmoor.quorum.chat".into()),
        };
        out.push(notice(&QUORUM, when, now, content, 1));
    }

    const COMMENTS: &[(&str, &str, &str)] = &[
        ("daniel", "commented on", "Q4 Plan"),
        ("lena", "mentioned you in", "Launch announcement draft"),
        ("priya", "resolved 3 comments in", "Atlas rate-card spec"),
        ("tom", "commented on", "Onboarding flow v3"),
        ("marcus", "shared", "Board deck numbers (Nov)"),
        ("ines", "mentioned you in", "Q3 on-time delivery analysis"),
        ("jordan", "commented on", "Harborlight proposal"),
        (
            "chloe",
            "assigned you a comment in",
            "Support macros for Atlas",
        ),
    ];
    for (index, (who, verb, doc)) in COMMENTS.iter().enumerate() {
        let who = cast::get(who);
        let when = at(
            now,
            i64::try_from(index).unwrap_or(0) * 3 + rng.between(0, 2),
            i8::try_from(rng.between(9, 18)).unwrap_or(9),
            i8::try_from(rng.between(0, 59)).unwrap_or(0),
        );
        if when > *now {
            continue;
        }
        let content = Content {
            subject: format!("{} {verb} \"{doc}\"", who.name()),
            paragraphs: vec![
                format!("{} {verb} \"{doc}\"", who.name()),
                "\"I think this section needs one more pass. Can we tighten the first paragraph?\""
                    .to_owned(),
            ],
            cta: ("Open in Docket", "https://docket.so/d/brightmoor".into()),
        };
        out.push(notice(&DOCKET, when, now, content, 2));
    }

    const PARCELS: &[(i64, &str, &str)] = &[
        (
            0,
            "Out for delivery: your package from Field & Fell",
            "Your package is out for delivery and should arrive by 8pm today.",
        ),
        (
            1,
            "Shipped: Order PP-48213-77",
            "Your order has shipped. Estimated delivery: tomorrow.",
        ),
        (
            6,
            "Delivered: your package was left at the front desk",
            "Your package from Kettle & Crumb was delivered at 2:14pm.",
        ),
        (
            7,
            "Shipped: Order PP-47790-12",
            "Your order has shipped. Estimated delivery: Thursday.",
        ),
        (
            19,
            "Delivered: your package was handed to resident",
            "Your package from Linen House was delivered at 11:02am.",
        ),
    ];
    for (days, subject, line) in PARCELS {
        let when = at(now, *days, 7, 50);
        if when > *now {
            continue;
        }
        let content = Content {
            subject: (*subject).to_owned(),
            paragraphs: vec![
                (*line).to_owned(),
                format!(
                    "Tracking number: 1PX{}",
                    400_000_000 + rng.below(99_999_999)
                ),
            ],
            cta: ("Track package", "https://parcelpoint.com/track".into()),
        };
        out.push(notice(&PARCELPOINT, when, now, content, 1));
    }

    const RECEIPTS: &[(i64, &str, &str)] = &[
        (2, "Receipt matched: Taxi", "$38.50"),
        (10, "Expense approved: Customer dinner, Chicago", "$412.80"),
        (16, "October expense report submitted", "$1,284.16"),
        (31, "Receipt matched: Northstar Air LGA–ORD", "$318.40"),
        (44, "Expense approved: Team lunch", "$186.25"),
    ];
    for (days, subject, amount) in RECEIPTS {
        let when = at(now, *days, 12, 5);
        let content = Content {
            subject: format!("{subject} ({amount})"),
            paragraphs: vec![
                format!("{subject} ({amount})."),
                "No action needed.".to_owned(),
            ],
            cta: ("View in Ledgerly", "https://ledgerly.app/expenses".into()),
        };
        let mut thread = notice(&LEDGERLY, when, now, content, 0);
        thread.state.labels.push("Receipts");
        out.push(thread);
    }

    const BANK: &[(i64, &str, &str)] = &[
        (
            1,
            "Large transaction alert",
            "A payment of $3,395.00 to Westbrook Realty was made from your checking account ending in 4417.",
        ),
        (
            3,
            "Your November statement is ready",
            "Your statement for the account ending in 4417 is now available in online banking.",
        ),
        (
            12,
            "Payment received. Thank you!",
            "We received your credit card payment of $2,148.33.",
        ),
        (
            34,
            "Your October statement is ready",
            "Your statement for the account ending in 4417 is now available in online banking.",
        ),
    ];
    for (days, subject, line) in BANK {
        let when = at(now, *days, 5, 30);
        let content = Content {
            subject: (*subject).to_owned(),
            paragraphs: vec![
                (*line).to_owned(),
                "For your security, we will never ask for your password by email.".to_owned(),
            ],
            cta: (
                "Sign in to online banking",
                "https://harborbank.com/login".into(),
            ),
        };
        let mut thread = notice(&HARBOR_BANK, when, now, content, 2);
        thread.state.important = true;
        out.push(thread);
    }

    // A flight itinerary with a PDF, filed under Travel.
    let when = at(now, 32, 15, 44);
    let content = Content {
        subject: "Your trip to Chicago: confirmation 7KQ2LM".into(),
        paragraphs: vec![
            "Your trip is confirmed. Confirmation code: 7KQ2LM".to_owned(),
            "Thu, Oct 15 · NS 412 · LGA 7:05am → ORD 8:41am".to_owned(),
            "Fri, Oct 16 · NS 419 · ORD 6:20pm → LGA 9:38pm".to_owned(),
        ],
        cta: (
            "Manage trip",
            "https://northstarair.com/trips/7KQ2LM".into(),
        ),
    };
    let mut trip = notice(&RESERVATIONS, when, now, content, 0);
    trip.posts[0].attachments.push(Attachment {
        filename: "Itinerary_7KQ2LM.pdf".into(),
        content_type: "application/pdf".into(),
        data: assets::pdf(
            "Northstar Air - Itinerary 7KQ2LM",
            &[
                "Passenger: MAYA OKAFOR",
                "NS 412 LGA-ORD Thu Oct 15 7:05am",
                "NS 419 ORD-LGA Fri Oct 16 6:20pm",
                "Seat 12A / 14C",
            ],
        ),
        content_id: None,
    });
    trip.state.labels.push("Travel");
    trip.state.important = true;
    out.push(trip);
}

fn social_and_forums(now: &Zoned, rng: &mut Rng, out: &mut Vec<Thread>) {
    const SOCIAL: &[&str] = &[
        "Grace Liu and 3 others viewed your profile",
        "You appeared in 22 searches this week",
        "Congratulate Tom Becker on 3 years at Brightmoor",
        "Mei Watanabe shared a post: Hollis Conference 2027 speakers",
        "Rachel Kim endorsed you for Product Management",
        "Your post is getting attention: 1,204 impressions",
        "New job openings at companies you follow",
        "Amara Nwosu wants to connect",
    ];
    for (index, subject) in SOCIAL.iter().enumerate() {
        let when = at(
            now,
            i64::try_from(index).unwrap_or(0) * 4 + rng.between(0, 3),
            18,
            i8::try_from(rng.between(0, 59)).unwrap_or(0),
        );
        if when > *now {
            continue;
        }
        let content = Content {
            subject: (*subject).to_owned(),
            paragraphs: vec![
                (*subject).to_owned(),
                "See who's engaging with your work on Linkwise.".to_owned(),
            ],
            cta: ("Open Linkwise", "https://linkwise.com/notifications".into()),
        };
        out.push(notice(&LINKWISE, when, now, content, 0));
    }
    const TOPICS: &[&str] = &[
        "Weekly digest: 14 new discussions",
        "Re: How do you run beta programs for B2B?",
        "Re: Tools for writing product specs in 2026",
        "Weekly digest: 9 new discussions",
        "Re: Measuring activation for multi-seat products",
        "Re: Pricing for ops software, per seat or per load?",
    ];
    for (index, topic) in TOPICS.iter().enumerate() {
        let when = at(now, i64::try_from(index).unwrap_or(0) * 5 + 1, 9, 0);
        let content = Content {
            subject: format!("[ProductOps] {topic}"),
            paragraphs: vec![
                format!("[ProductOps] {topic}"),
                "\"We stopped calling it a beta and started calling it a design partnership. Expectations got a lot clearer on both sides.\"".to_owned(),
            ],
            cta: ("View discussion", "https://community.productops.org".into()),
        };
        let mut thread = notice(&PRODUCTOPS, when, now, content, 0);
        thread.list = Some(List {
            id: "ProductOps Community <digest.community.productops.org>".into(),
            unsubscribe: "https://community.productops.org/unsubscribe".into(),
        });
        out.push(thread);
    }
}

fn spam(now: &Zoned, rng: &mut Rng, out: &mut Vec<Thread>) {
    const SPAM: &[(&str, &str, &str)] = &[
        (
            "Prize Center",
            "winner@prize-center-rewards.top",
            "Congratulations! You've been selected for a $1,000 gift card",
        ),
        (
            "Account Security",
            "security@acct-verify-alerts.info",
            "URGENT: Your mailbox will be suspended in 24 hours",
        ),
        (
            "Accounts Payable",
            "ap@invoices-overdue-notice.biz",
            "Re: Invoice #88123 overdue, final notice",
        ),
        (
            "Crypto Desk",
            "alpha@moonshot-yield.click",
            "This token is up 4,000% 🚀🚀🚀 don't miss the next one",
        ),
        (
            "Delivery Service",
            "noreply@parcel-redelivery-center.xyz",
            "Your package could not be delivered, action required",
        ),
        (
            "Work From Home",
            "jobs@remote-income-now.top",
            "Make $5,000/week from home, no experience needed",
        ),
        (
            "Pharmacy Direct",
            "deals@cheap-meds-online.shop",
            "Your discount prescription refill is ready",
        ),
        (
            "Lottery Commission",
            "claims@intl-lottery-board.org.ng",
            "Claim your unclaimed winnings of $2,500,000.00",
        ),
    ];
    for (index, (name, email, subject)) in SPAM.iter().enumerate() {
        let when = at(
            now,
            i64::try_from(index).unwrap_or(0) * 3 + rng.between(0, 2),
            i8::try_from(rng.between(0, 23)).unwrap_or(3),
            i8::try_from(rng.between(0, 59)).unwrap_or(0),
        );
        if when > *now {
            continue;
        }
        let text = format!(
            "Dear Customer,\n\n{subject}. Click the link below within 24 hours to claim.\n\nhttp://{}/claim?id={}",
            email.split_once('@').map_or("", |(_, d)| d),
            rng.below(1_000_000)
        );
        let html = format!(
            "<html><body style=\"font-family:Arial\"><p>Dear Customer,</p><p><b>{}</b></p><p><a href=\"http://{}/claim\">CLICK HERE TO CLAIM</a></p></body></html>",
            escape(subject),
            email.split_once('@').map_or("", |(_, d)| d)
        );
        out.push(Thread {
            subject: (*subject).to_owned(),
            posts: vec![Post {
                from: Mailbox::new(*name, *email),
                to: vec![cast::me().mailbox()],
                cc: Vec::new(),
                text,
                html: Some(html),
                calendar: None,
                attachments: Vec::new(),
                at: when.timestamp(),
            }],
            state: State {
                spam: true,
                ..State::default()
            },
            list: None,
            quote: false,
        });
    }
}
