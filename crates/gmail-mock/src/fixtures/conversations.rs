//! Person-to-person threads: Maya's colleagues, customers, friends and family.

/// An attachment on one post of a script.
pub enum File {
    Pdf {
        name: &'static str,
        title: &'static str,
        lines: &'static [&'static str],
    },
    Png {
        name: &'static str,
        bars: &'static [u8],
    },
    Csv {
        name: &'static str,
        body: &'static str,
    },
}

pub struct Script {
    pub subject: &'static str,
    /// Days before "now", and local hour and minute, of the first message.
    pub start: (i64, i8, i8),
    /// Minutes between messages, as a range the generator draws from.
    pub pace: (i64, i64),
    pub to: &'static [&'static str],
    pub cc: &'static [&'static str],
    /// `(sender, body)` in order. Replies go to the previous sender and cc everyone else.
    pub posts: &'static [(&'static str, &'static str)],
    pub inbox: bool,
    pub unread: usize,
    pub starred: bool,
    pub labels: &'static [&'static str],
    /// Attachments, by post index.
    pub files: &'static [(usize, File)],
}

const BASE: Script = Script {
    subject: "",
    start: (1, 9, 0),
    pace: (15, 150),
    to: &[],
    cc: &[],
    posts: &[],
    inbox: false,
    unread: 0,
    starred: false,
    labels: &[],
    files: &[],
};

pub static SCRIPTS: &[Script] = &[
    Script {
        subject: "Q4 planning doc — comments by Thursday?",
        start: (1, 8, 47),
        to: &["maya"],
        cc: &["priya", "lena"],
        posts: &[
            (
                "daniel",
                "Hi all,\n\nI pulled the Q4 plan into one doc so we stop working across three. The big open question is still whether Atlas GA slips to January or we ship with the reduced carrier list in December.\n\nCan you leave comments by Thursday EOD? I'd like to lock it before the board prep on Monday.\n\nThanks,\nDaniel",
            ),
            (
                "maya",
                "Thanks Daniel. I'll go through it tonight.\n\nMy vote is December with the reduced list. Calloway and Harborlight only need the top 12 carriers, and a January date pushes us into the renewal window for both.\n\nMaya",
            ),
            (
                "priya",
                "Agree with Maya on December. Engineering-wise the reduced list is fine; the risk is the rate-card import, which we can feature-flag.\n\nI left a few comments on the staffing section. We're one backend engineer short in the plan as written.",
            ),
            (
                "daniel",
                "Great, December it is unless Lena sees a problem on the launch side. Priya, let's talk staffing in our 1:1 tomorrow.",
            ),
        ],
        inbox: true,
        unread: 1,
        labels: &["Projects/Atlas Launch"],
        ..BASE
    },
    Script {
        subject: "Atlas beta: feedback from our dispatch team",
        start: (3, 10, 12),
        pace: (40, 300),
        to: &["maya"],
        cc: &["jordan", "dev"],
        posts: &[
            (
                "rachel",
                "Hi Maya,\n\nWe've had Atlas running in parallel with our current routing for two weeks now. Overall the team likes it a lot. Suggested routes are noticeably tighter on the Chicago–Milwaukee lanes.\n\nTwo issues came up:\n\n1. Multi-stop loads with a dock appointment after 4pm sometimes get scheduled for the next morning even when there's capacity.\n2. The driver hours-of-service warning shows up after the dispatcher confirms, not before.\n\nScreenshot of the first one attached. Happy to get on a call.\n\nRachel",
            ),
            (
                "maya",
                "Rachel, thank you, this is exactly the feedback we need.\n\n#2 is a known ordering bug and the fix ships Thursday. #1 is new to us. Looping in Priya's team; could you send two or three load IDs where you saw it?\n\nJordan will set up a call for early next week.\n\nMaya",
            ),
            (
                "dev",
                "Load IDs: CF-88213, CF-88240 and CF-88302. All three had the appointment set via the API rather than the UI, if that helps.",
            ),
            (
                "rachel",
                "Also, our VP saw the demo on Friday and asked whether Atlas can handle cross-dock transfers. Not urgent, but it'll come up at renewal.",
            ),
        ],
        inbox: true,
        unread: 2,
        starred: true,
        labels: &["Clients/Calloway"],
        files: &[(
            0,
            File::Png {
                name: "atlas-next-morning-bug.png",
                bars: &[72, 64, 88, 30, 55, 91],
            },
        )],
    },
    Script {
        subject: "MSA v3 redlines",
        start: (2, 14, 5),
        pace: (60, 240),
        to: &["maya"],
        cc: &["daniel"],
        posts: &[
            (
                "ben",
                "Maya,\n\nAttached is the redline of Calloway's MSA v3 against our template. Most of their changes are fine. The ones that need a business decision:\n\n- §7.2: they want uncapped liability for data breaches. I recommend countering with 3x annual fees.\n- §9.1: 60-day termination for convenience. Our standard is 90.\n- Exhibit B: an SLA credit of 10% per hour of downtime, which is well outside market.\n\nLet me know how you'd like to respond.\n\nBest,\nBen\n\nBen Ortiz | Associate\nMarlow & Finch LLP",
            ),
            (
                "daniel",
                "3x on §7.2 works. I'd accept 60 days on §9.1 if they drop the SLA credits to our standard schedule. Maya, does that match what you heard from Rachel?",
            ),
        ],
        inbox: true,
        unread: 0,
        labels: &["Clients/Calloway"],
        files: &[(
            0,
            File::Pdf {
                name: "Calloway_MSA_v3_redline.pdf",
                title: "Master Services Agreement (v3) - Redline",
                lines: &[
                    "Brightmoor, Inc. and Calloway Freight Co.",
                    "Section 7.2 Limitation of Liability",
                    "Section 9.1 Termination for Convenience",
                    "Exhibit B Service Levels",
                    "Draft - privileged and confidential",
                ],
            },
        )],
        ..BASE
    },
    Script {
        subject: "Launch page illustrations — first sketches",
        start: (6, 11, 30),
        pace: (120, 900),
        to: &["maya"],
        cc: &["tom"],
        posts: &[
            (
                "felix",
                "Hi Maya and Tom,\n\nHere are three directions for the hero illustration. A is the isometric depot, B is the map with route lines, C is the more playful truck-and-parcels idea.\n\nI'd lean B, it carries the \"routing\" idea without words. Let me know which way you want to go and I'll take it to color by Friday.",
            ),
            (
                "tom",
                "B for sure. Could the route lines be in our blue, with the map in warm grays? The current palette feels a bit cold next to the product screenshots.",
            ),
            (
                "maya",
                "Agree with Tom. B, warmer map. Felix, could you also do a square crop for social?",
            ),
            (
                "felix",
                "Will do, both. Color version with the square crop coming Thursday.",
            ),
            (
                "felix",
                "As promised, color pass attached. The square crop is the second frame.",
            ),
        ],
        inbox: false,
        labels: &["Projects/Atlas Launch"],
        files: &[
            (
                0,
                File::Png {
                    name: "hero-directions-ABC.png",
                    bars: &[45, 80, 60],
                },
            ),
            (
                4,
                File::Png {
                    name: "hero-B-color.png",
                    bars: &[30, 50, 70, 90],
                },
            ),
        ],
        ..BASE
    },
    Script {
        subject: "Offsite catering — Hudson, Dec 9–11",
        start: (9, 13, 15),
        pace: (90, 600),
        to: &["maya"],
        cc: &["ruth"],
        posts: &[
            (
                "sofia",
                "Hi Maya,\n\nThanks for reaching out! We'd love to cater your team's offsite. For 22 people over three days I'd suggest a breakfast spread each morning, family-style lunches, and one plated dinner on the 10th.\n\nI've attached a sample menu. Do you have any dietary restrictions I should plan around?\n\nWarmly,\nSofia",
            ),
            (
                "maya",
                "This looks wonderful. Ruth has the dietary list, I'll let her send it over. The plated dinner on the 10th is a yes.",
            ),
            (
                "ruth",
                "Hi Sofia, we have 3 vegetarian, 1 vegan, 2 gluten-free and one severe tree-nut allergy (Oliver). Could you confirm the kitchen can handle the nut allergy?",
            ),
            (
                "sofia",
                "Absolutely, we run a nut-free kitchen for events with severe allergies. I'll send the final quote by Friday.",
            ),
        ],
        inbox: false,
        files: &[(
            0,
            File::Pdf {
                name: "Romano_Catering_Sample_Menu.pdf",
                title: "Romano Catering - Sample Event Menu",
                lines: &[
                    "Breakfast: seasonal fruit, yogurt, house granola, frittata",
                    "Lunch: roasted vegetables, farro salad, herb chicken",
                    "Dinner: braised short rib or wild mushroom risotto",
                    "Dessert: olive oil cake, poached pears",
                ],
            },
        )],
        ..BASE
    },
    Script {
        subject: "Thanksgiving 🦃",
        start: (4, 19, 40),
        pace: (60, 400),
        to: &["maya", "emeka"],
        posts: &[
            (
                "ngozi",
                "Hi my loves,\n\nDaddy and I are doing Thanksgiving at the house this year. Please tell me by this weekend if you're coming so I know how much to cook. Emeka, are you bringing Tolu?\n\nMaya, can you make your sweet potato thing again? Everyone asked about it last year.\n\nLove,\nMommy",
            ),
            (
                "emeka",
                "We're in! Tolu says she'll bring dessert. Arriving Wednesday night.",
            ),
            (
                "maya",
                "Coming Wednesday too, taking the 5:10 train. And yes, sweet potatoes are on me 🍠",
            ),
            (
                "ngozi",
                "Wonderful!! I'll pick you up from the station. Don't forget a warm coat, it's supposed to be cold.",
            ),
        ],
        inbox: true,
        unread: 1,
        labels: &["Family"],
        ..BASE
    },
    Script {
        subject: "dinner thurs?",
        start: (2, 18, 22),
        pace: (20, 120),
        to: &["maya"],
        posts: &[
            (
                "jess",
                "are you free thursday?? that new place on Atlantic finally opened, the one with the wood oven. I can book 7:30",
            ),
            (
                "maya",
                "YES. 7:30 is perfect. I might be 10 min late, board prep week",
            ),
            (
                "jess",
                "booked! table under Moreno. no laptops at dinner 😤",
            ),
        ],
        inbox: true,
        unread: 1,
        ..BASE
    },
    Script {
        subject: "Lease renewal — Apt 14B",
        start: (12, 10, 2),
        pace: (300, 1800),
        to: &["maya"],
        posts: &[
            (
                "paul",
                "Dear Ms. Okafor,\n\nYour lease for 14B at 220 Bergen Street expires on January 31, 2027. We are pleased to offer a 12-month renewal at $3,450/month (a 3.0% increase) or a 24-month renewal at $3,395/month.\n\nThe renewal agreement is attached. Please let us know your decision by December 15.\n\nBest regards,\nPaul Durand\nWestbrook Realty",
            ),
            (
                "maya",
                "Hi Paul,\n\nThanks for sending this over. I'd like to go with the 24-month option. Could you confirm the radiator in the bedroom will be serviced before winter? It was making noise again last week.\n\nBest,\nMaya",
            ),
            (
                "paul",
                "Noted on the 24-month term. I've put in a work order for the radiator; the super will reach out to schedule this week.",
            ),
        ],
        inbox: false,
        files: &[(
            0,
            File::Pdf {
                name: "14B_Lease_Renewal_2027.pdf",
                title: "Lease Renewal Agreement - Unit 14B",
                lines: &[
                    "220 Bergen Street, Brooklyn, NY",
                    "Term options: 12 months or 24 months",
                    "Commencement: February 1, 2027",
                    "Please sign and return by December 15, 2026",
                ],
            },
        )],
        ..BASE
    },
    Script {
        subject: "Debrief: Senior Product Designer (Aiko)",
        start: (1, 16, 30),
        pace: (20, 90),
        to: &["maya", "tom"],
        posts: &[
            (
                "zara",
                "Thanks both for making time for Aiko's onsite. Could you drop your scorecards in by tomorrow noon? She has another offer with a deadline on Friday, so we need to move quickly if it's a yes.",
            ),
            (
                "tom",
                "Strong yes from me. Her portfolio walkthrough was the best we've seen this cycle, and the whiteboard exercise showed she can simplify a messy flow without losing the edge cases.",
            ),
            (
                "maya",
                "Yes from me too. Some concern about how much she's worked directly with engineering, but she asked the right questions about constraints. Scorecard is in.",
            ),
        ],
        inbox: true,
        unread: 0,
        starred: true,
        labels: &["Hiring"],
        ..BASE
    },
    Script {
        subject: "Pricing page v2 — mockups",
        start: (5, 9, 50),
        to: &["maya", "tom"],
        posts: &[
            (
                "lena",
                "Morning! Attached are the v2 pricing mockups. Main changes: annual toggle is on by default, the Growth tier is highlighted, and the comparison table is collapsed behind \"See all features\".\n\nWould love feedback before Tom starts the build.",
            ),
            (
                "maya",
                "Looks great. One worry: hiding the comparison table hurts the ops buyers who want to check SSO and audit logs before they talk to sales. Could we keep the security rows visible?",
            ),
            (
                "tom",
                "Easy change, I'll pull the security rows into their own always-visible section.",
            ),
            ("lena", "Love it. Shipping that version 👍"),
        ],
        inbox: false,
        labels: &["Projects/Atlas Launch"],
        files: &[(
            0,
            File::Png {
                name: "pricing-v2.png",
                bars: &[35, 70, 50],
            },
        )],
        ..BASE
    },
    Script {
        subject: "Board deck — draft 2",
        start: (0, 7, 12),
        to: &["maya", "marcus"],
        posts: &[(
            "daniel",
            "Draft 2 attached. Maya, the product section still has placeholder numbers on slide 9 (Atlas adoption). Marcus, can you double-check the burn multiple on slide 14?\n\nFinal version needs to go out Sunday night.",
        )],
        inbox: true,
        unread: 1,
        files: &[(
            0,
            File::Pdf {
                name: "Brightmoor_Board_Deck_Nov2026_v2.pdf",
                title: "Brightmoor Board Meeting - November 2026",
                lines: &[
                    "1. Highlights and lowlights",
                    "2. Q3 results vs plan",
                    "3. Atlas: beta results and launch plan",
                    "4. Hiring and team",
                    "5. Financials and runway",
                    "Draft 2 - confidential",
                ],
            },
        )],
        ..BASE
    },
    Script {
        subject: "Brightmoor — November investor update",
        start: (8, 17, 5),
        pace: (300, 1500),
        to: &["grace"],
        posts: &[
            (
                "maya",
                "Hi Grace,\n\nDaniel asked me to send the product half of this month's update while he's traveling.\n\nHighlights:\n- Atlas beta is live with 4 customers; Calloway has moved 30% of lanes onto it.\n- Median dispatch time is down 41% for beta users.\n- We hired two engineers and have an offer out for a senior designer.\n\nAsk: we're looking for intros to ops leaders at mid-size 3PLs in the Southeast.\n\nBest,\nMaya",
            ),
            (
                "grace",
                "This is a great update, Maya. Congrats on the dispatch time numbers. I'll make two intros this week, both run ops at 3PLs out of Atlanta.",
            ),
        ],
        inbox: false,
        ..BASE
    },
    Script {
        subject: "2026 tax documents checklist",
        start: (15, 11, 0),
        pace: (600, 2000),
        to: &["maya"],
        posts: &[
            (
                "carlos",
                "Hi Maya,\n\nAhead of year end, here is the checklist of documents we'll need for your 2026 return. The CSV lists each item and whether we already have it on file.\n\nIf you exercised any options this year, please send the 3921 forms as soon as they're available.\n\nThank you,\nCarlos Mendes\nPemberton CPA",
            ),
            (
                "maya",
                "Thanks Carlos. No exercises this year. I'll upload the brokerage statements in January once they're out.",
            ),
        ],
        inbox: false,
        files: &[(
            0,
            File::Csv {
                name: "2026_document_checklist.csv",
                body: "document,status,notes\nW-2 (Brightmoor),pending,Expected late January\n1099-INT (Harbor Bank),pending,\n1099-B (brokerage),pending,\nCharitable receipts,on file,3 receipts received\nHome office expenses,not needed,\n",
            },
        )],
        ..BASE
    },
    Script {
        subject: "On-time delivery by region — Q3",
        start: (7, 15, 20),
        to: &["maya"],
        posts: &[
            (
                "maya",
                "Hi Inês, for the board deck could you pull on-time delivery rates by region for Q3, split by Atlas beta vs. everyone else? Doesn't need to be pretty.",
            ),
            (
                "ines",
                "Here you go. Midwest is where Atlas helps the most (+9 points). The Southeast sample is small, only two beta customers, so I'd caveat that one.",
            ),
            ("maya", "Perfect, thank you! Exactly what I needed."),
        ],
        inbox: false,
        files: &[(
            1,
            File::Csv {
                name: "otd_by_region_q3.csv",
                body: "region,cohort,loads,on_time_pct\nMidwest,atlas_beta,4120,94.2\nMidwest,control,18230,85.1\nNortheast,atlas_beta,2875,91.7\nNortheast,control,15112,87.9\nSoutheast,atlas_beta,610,89.4\nSoutheast,control,9874,86.3\nWest,atlas_beta,1930,92.8\nWest,control,12650,88.0\n",
            },
        )],
        ..BASE
    },
    Script {
        subject: "Intro: Yusuf (Tinkerform) <> Amara (Quill & Oak)",
        start: (10, 12, 45),
        pace: (60, 600),
        to: &["yusuf", "amara"],
        posts: &[
            (
                "maya",
                "Yusuf, meet Amara. Amara runs Quill & Oak and went through exactly the warehouse-software selection you're starting. Amara, Yusuf is building Tinkerform and is looking for advice on picking a WMS.\n\nI'll let you two take it from here!\n\nMaya",
            ),
            (
                "amara",
                "Thanks Maya! Yusuf, happy to help. I have time Thursday or Friday afternoon, here's my calendar link: (moving Maya to bcc)",
            ),
            (
                "yusuf",
                "Thank you both! Amara, Friday 2pm works, I'll grab it.",
            ),
        ],
        inbox: false,
        ..BASE
    },
    Script {
        subject: "Speaking at Hollis Conference 2027",
        start: (11, 9, 15),
        pace: (300, 3000),
        to: &["maya"],
        posts: &[
            (
                "mei",
                "Dear Maya,\n\nI'm the program chair for Hollis Conference 2027 (April 14–16, Chicago). We'd be delighted to have you give a 30-minute talk on building products for operations teams. Your post on designing for dispatchers was widely shared among our attendees.\n\nWould you be interested?\n\nWith best wishes,\nMei Watanabe",
            ),
            (
                "maya",
                "Hi Mei,\n\nThank you, I'd be honored! April works. Could you share the format and the deadline for the abstract?\n\nMaya",
            ),
            (
                "mei",
                "Wonderful! Abstracts are due January 10. The speaker guide is attached. We cover travel and two nights at the conference hotel.",
            ),
        ],
        inbox: true,
        unread: 1,
        files: &[(
            2,
            File::Pdf {
                name: "Hollis2027_Speaker_Guide.pdf",
                title: "Hollis Conference 2027 - Speaker Guide",
                lines: &[
                    "Dates: April 14-16, 2027",
                    "Venue: Hollis Hall, Chicago",
                    "Talk length: 30 minutes including Q&A",
                    "Abstract deadline: January 10, 2027",
                ],
            },
        )],
        ..BASE
    },
    Script {
        subject: "Harborlight pilot — next steps",
        start: (3, 14, 40),
        pace: (45, 200),
        to: &["jordan"],
        cc: &["maya"],
        posts: &[
            (
                "theo",
                "Hi Jordan,\n\nThe team at our Seattle roastery wrapped the two-week pilot. Store deliveries were consolidated from 11 runs a week to 8, which is real money for us.\n\nNext step on our side is a proposal for all 14 locations. Can you send pricing for that?\n\nTheo",
            ),
            (
                "jordan",
                "Theo, that's fantastic. I'll have a proposal over by Wednesday. Maya, any roadmap items we should mention? They asked about cold-chain tracking during the pilot.",
            ),
            (
                "maya",
                "Cold-chain alerts are on the Q1 roadmap. Happy to join a call with Theo to walk through it.",
            ),
        ],
        inbox: true,
        unread: 0,
        ..BASE
    },
    Script {
        subject: "Ridgeline Health: SSO provisioning question",
        start: (5, 13, 5),
        pace: (30, 240),
        to: &["chloe"],
        posts: &[
            (
                "nadia",
                "Hi Chloé, our IT team is setting up SSO and is asking whether Brightmoor supports SCIM provisioning, or only SAML login. Thanks!",
            ),
            (
                "chloe",
                "Hi Nadia! SAML login is supported today; SCIM is in beta. Looping in Maya, who owns the roadmap, for timing.",
            ),
            (
                "maya",
                "Hi Nadia, SCIM is in a closed beta now and we can add Ridgeline to it next week. GA is planned for February.",
            ),
            ("nadia", "That works, please add us. Thank you both!"),
        ],
        inbox: false,
        ..BASE
    },
    Script {
        subject: "Saturday hike? 🥾",
        start: (6, 20, 10),
        pace: (30, 300),
        to: &["maya", "hannah"],
        posts: &[
            (
                "liam",
                "Forecast looks clear for Saturday. Breakneck Ridge? Train leaves Grand Central at 8:43.",
            ),
            (
                "hannah",
                "I'm in but I vote for something less vertical, my knee is still unhappy. Cold Spring to Little Stony Point?",
            ),
            ("maya", "Count me in for either! I'll bring snacks."),
            (
                "liam",
                "Little Stony Point it is. See you under the clock at 8:30.",
            ),
        ],
        inbox: false,
        ..BASE
    },
    Script {
        subject: "1:1 agenda",
        start: (0, 8, 2),
        to: &["maya"],
        posts: &[(
            "priya",
            "Few things for today:\n\n- Backend hiring: I'd like to open a second req now rather than in January\n- Atlas rate-card import, feature flag plan\n- On-call rotation for the launch week\n\nAnything you want to add?",
        )],
        inbox: true,
        unread: 1,
        ..BASE
    },
    Script {
        subject: "Incident review: routing delays on Nov 12",
        start: (4, 16, 10),
        pace: (20, 180),
        to: &["priya", "maya"],
        cc: &["oliver", "chloe"],
        posts: &[
            (
                "hiro",
                "Summary of Thursday's incident:\n\n- 14:02–14:49 ET: route suggestions took 30–90s instead of <2s\n- Cause: the carrier availability cache expired all at once after a deploy and every request went to the upstream API\n- Impact: 37 customers saw slow suggestions; no data loss\n\nAction items:\n1. Jitter the cache TTL (done)\n2. Add a circuit breaker around the availability API (Oliver, this week)\n3. Status page update template for partial degradation (Chloé)\n\nFull write-up in the doc.",
            ),
            (
                "chloe",
                "Thanks Hiro. Two customers emailed support during the window; I've replied to both with the summary.",
            ),
            (
                "maya",
                "Thanks for the fast write-up. Can we make sure the circuit breaker lands before the Atlas launch? Same failure during launch week would hurt.",
            ),
            ("oliver", "It'll be in staging Wednesday and prod Friday."),
        ],
        inbox: false,
        ..BASE
    },
    Script {
        subject: "Analytics dashboard access",
        start: (13, 11, 22),
        to: &["maya"],
        posts: &[
            (
                "oliver",
                "Hey Maya, could you give me viewer access to the product analytics workspace? I want to check the event volume before we change the tracking schema.",
            ),
            ("maya", "Done! You should have an invite."),
            ("oliver", "Got it, thanks!"),
        ],
        inbox: false,
        ..BASE
    },
    Script {
        subject: "Customer story draft — Calloway Freight",
        start: (2, 10, 30),
        to: &["maya"],
        posts: &[
            (
                "lena",
                "First draft of the Calloway customer story is in the doc. I need a quote from you about why we built Atlas, two or three sentences. Rachel has already approved her quotes.",
            ),
            (
                "maya",
                "Here you go: \"Dispatchers were making hundreds of small decisions a day with no help from their software. Atlas does the tedious math so they can focus on the calls only a person can make.\"",
            ),
            ("lena", "Perfect, that's going in as is."),
        ],
        inbox: false,
        labels: &["Projects/Atlas Launch"],
        ..BASE
    },
    Script {
        subject: "October expense report",
        start: (16, 9, 40),
        pace: (120, 900),
        to: &["maya"],
        posts: &[
            (
                "marcus",
                "Hi Maya, your October expense report is missing receipts for the Chicago customer dinner ($412.80) and one taxi ($38.50). Could you upload them by Friday so we can close the month?",
            ),
            ("maya", "Uploaded both. Sorry about that!"),
            ("marcus", "All set, thanks."),
        ],
        inbox: false,
        labels: &["Receipts"],
        ..BASE
    },
    Script {
        subject: "Coffee next week?",
        start: (1, 12, 5),
        to: &["maya"],
        posts: &[(
            "grace",
            "Maya, I'm in New York Tuesday through Thursday next week. Would love to grab coffee and hear more about how Atlas is going. Any morning work?",
        )],
        inbox: true,
        unread: 1,
        ..BASE
    },
    Script {
        subject: "Design crit moved to 3pm",
        start: (0, 8, 31),
        to: &["maya", "priya"],
        posts: &[(
            "tom",
            "Heads up: today's design crit moves to 3pm, Daniel needed the 1pm slot. Same room.",
        )],
        inbox: true,
        unread: 1,
        ..BASE
    },
    Script {
        subject: "Your birthday present 🎁",
        start: (20, 21, 4),
        pace: (30, 240),
        to: &["maya"],
        posts: &[
            (
                "emeka",
                "Ok I need help. Tolu's birthday is in 3 weeks and I have no ideas. You're good at this. Budget ~$150.",
            ),
            (
                "maya",
                "She mentioned wanting to learn pottery! There's a 4-week wheel class in Fort Greene. Or the cookbook from that Lagos restaurant she loved.",
            ),
            ("emeka", "POTTERY. You're a genius. Booking it now."),
        ],
        inbox: false,
        labels: &["Family"],
        ..BASE
    },
    Script {
        subject: "Team offsite: Hudson, Dec 9–11 🏔️",
        start: (5, 15, 0),
        pace: (10, 120),
        to: &[
            "maya", "daniel", "priya", "tom", "lena", "jordan", "hiro", "chloe", "marcus", "ines",
            "oliver", "zara", "ruth",
        ],
        posts: &[
            (
                "sam",
                "Hi everyone,\n\nThe offsite is booked! We'll be at the Millbrook Inn in Hudson from Tuesday Dec 9 through Thursday Dec 11.\n\n- Train: the 8:15 from Penn Station on the 9th (tickets coming from Ruth)\n- Rooms are single occupancy\n- Agenda draft is in the shared folder; add topics by Dec 1\n\nPlease reply with any dietary needs or accessibility requests.\n\nSam",
            ),
            (
                "hiro",
                "Can we have a session on the 2027 platform roadmap? Happy to prep it with Priya.",
            ),
            (
                "lena",
                "+1 to Hiro's idea. Also, is there a plan for the evening of the 10th? Some of us are thinking board games 🎲",
            ),
            (
                "oliver",
                "Reminder for the caterer: tree-nut allergy (me). Thanks!",
            ),
            (
                "daniel",
                "Love the energy. Platform roadmap is on the agenda for the morning of the 10th. Board games are mandatory.",
            ),
            (
                "ines",
                "Could someone share the address for folks driving? I'll bring a car with 3 free seats.",
            ),
            (
                "sam",
                "Millbrook Inn, 28 Warren St, Hudson NY. Ines, thank you! I'll set up a carpool sheet.",
            ),
        ],
        inbox: true,
        unread: 2,
        ..BASE
    },
    Script {
        subject: "Atlas launch checklist",
        start: (14, 10, 0),
        pace: (60, 900),
        to: &["priya", "lena", "tom", "hiro"],
        cc: &["daniel", "chloe", "oliver"],
        posts: &[
            (
                "maya",
                "Kicking off the launch checklist thread so we have one place for status. Launch target is Tuesday Dec 2.\n\nOwners:\n- Engineering readiness: Priya\n- Launch page + announcement: Lena\n- In-product onboarding: Tom\n- Support docs + macros: Chloé\n- Load testing: Hiro\n\nPlease reply here with status every Monday.",
            ),
            (
                "priya",
                "Engineering: rate-card import behind a flag, everything else feature complete. Bug burn-down is at 23 open, 4 of them launch-blocking.",
            ),
            (
                "lena",
                "Launch page copy is in review. Felix is on the illustrations. Press list is drafted, 18 outlets.",
            ),
            (
                "tom",
                "Onboarding flow designs are final. Build starts Wednesday.",
            ),
            (
                "hiro",
                "Load test plan is written. Running the first pass Thursday against staging at 3x peak.",
            ),
            (
                "chloe",
                "Support docs outline is done. Macros will follow once the onboarding copy is final.",
            ),
            (
                "hiro",
                "First load test results: p95 holds under 1.8s at 3x peak. At 5x the availability API becomes the bottleneck, which is the same thing Oliver is fixing with the circuit breaker.",
            ),
            (
                "daniel",
                "Great progress. Let's make sure pricing is final before the press list goes out. Lena, can you sync with Jordan?",
            ),
            (
                "lena",
                "Synced with Jordan, pricing is final. Growth tier stays at $49/seat.",
            ),
            (
                "priya",
                "Weekly status: 9 open bugs, 1 launch-blocking (the dock appointment scheduling issue Calloway reported). Hiro is on it.",
            ),
            (
                "tom",
                "Onboarding build is done and in staging. Would love a few people to run through it before Friday.",
            ),
            (
                "maya",
                "Ran through onboarding, really nice work Tom. Two nits: the \"Skip\" link is hard to see, and step 3 says \"carrier\" where it should say \"carrier group\".",
            ),
            ("tom", "Both fixed."),
            (
                "chloe",
                "Macros done. I also recorded a 3-minute walkthrough for the support team.",
            ),
            (
                "hiro",
                "Fixed the dock appointment bug. Root cause was a timezone conversion on API-created appointments. Calloway's three load IDs now schedule correctly.",
            ),
            (
                "oliver",
                "Circuit breaker is live in prod. Re-ran the 5x load test with Hiro: p95 1.9s, no upstream errors.",
            ),
            (
                "lena",
                "Launch page is staged behind a password. Announcement post goes out at 9am ET on Dec 2, emails at 10am.",
            ),
            (
                "priya",
                "Zero launch-blocking bugs. Code freeze is Friday at 5pm.",
            ),
            (
                "daniel",
                "This is a great team. Thank you all. Let's do a go/no-go Monday Dec 1 at 4pm.",
            ),
            ("maya", "Calendar invite for go/no-go is out. 🚀"),
        ],
        inbox: true,
        unread: 0,
        starred: true,
        labels: &["Projects/Atlas Launch"],
        ..BASE
    },
    Script {
        subject: "Book club: what's next? 📚",
        start: (25, 19, 30),
        pace: (40, 1400),
        to: &["jess", "liam", "hannah"],
        posts: &[
            (
                "maya",
                "Ok book club, we finished the lighthouse one. Nominations for next month?",
            ),
            (
                "jess",
                "I nominate the one with the orange cover everyone keeps posting about, the quiet sad robot one",
            ),
            (
                "hannah",
                "Can we do nonfiction for once? I've been wanting to read the one about the history of shipping containers",
            ),
            ("liam", "Hannah that is SUCH a Maya-coded pick"),
            ("maya", "I won't pretend I'm not interested 😅"),
            (
                "jess",
                "fine but only if we go somewhere with good wine to discuss it",
            ),
            ("hannah", "Deal. Containers book, wine bar. Who's hosting?"),
            ("liam", "I can host, I finally have enough chairs"),
            ("maya", "Liam has chairs!! Growth"),
            ("jess", "Date? I'm out of town the 6th–8th"),
            ("hannah", "How about Sunday the 14th?"),
            ("liam", "14th works"),
            ("maya", "14th works for me too. 4pm?"),
            ("jess", "4pm 👍 I'll bring cheese"),
            ("hannah", "I'll bring the wine since it was my pick"),
            ("liam", "Perfect. See you all on the 14th"),
        ],
        inbox: false,
        ..BASE
    },
    Script {
        subject: "Conference room for Thursday",
        start: (3, 11, 47),
        to: &["ruth"],
        posts: &[(
            "maya",
            "Hi Ruth, could you book the big conference room Thursday 2–4pm for the Calloway call? We'll have 6 people in the room and 3 remote. Thank you!",
        )],
        ..BASE
    },
    Script {
        subject: "Notes from the Calloway QBR",
        start: (9, 17, 33),
        to: &["daniel"],
        cc: &["jordan"],
        posts: &[(
            "maya",
            "Quick notes from today's QBR:\n\n- They're happy overall; NPS from their dispatch team went from 31 to 52\n- Biggest ask is cross-dock transfers\n- Renewal conversation starts in January; Rachel hinted at expanding to their Texas region\n- They want a named support contact for launch week\n\nI told them Chloé would be their contact.",
        )],
        ..BASE
    },
    Script {
        subject: "photos from saturday",
        start: (5, 21, 15),
        to: &["liam", "hannah"],
        posts: &[(
            "maya",
            "the view from the top!! sending the rest via shared album",
        )],
        files: &[(
            0,
            File::Png {
                name: "IMG_4021.png",
                bars: &[20, 40, 75, 95, 60, 35],
            },
        )],
        ..BASE
    },
];
