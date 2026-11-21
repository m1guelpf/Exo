//! The popups' copy, one line per entry; `_…_` is italic.

/// A send popup past 200 sends without a message of its own.
pub(super) const SEND_POOL: &[&[&str]] = &[
    &["Cheers to accomplishing more and saving time."],
    &["With your hands on the keyboard, no one can", "stop you."],
    &["Another task completed — keep going!"],
    &["You're a power sender! Relish in the glory."],
    &[
        "Bravo! Here's to being faster, more responsive, ",
        "and more effective.",
    ],
    &[
        "Nicely done! Reflect on all the messages",
        "you've conquered.",
    ],
    &[
        "Your job isn't just sending emails, but if it",
        "was — you deserve a raise.",
    ],
    &[
        "The pen is mightier than the sword, but your",
        "keyboard is mightier than the pen.",
    ],
    &["How does it feel to be this good at email?"],
    &[
        "Another milestone! Keep raising the bar for",
        "productivity.",
    ],
];

/// A Mark Done popup past 500 without a message of its own.
pub(super) const CLEAR_POOL: &[&[&str]] = &[
    &[
        "Well done! Every email cleared is a weight off",
        "of your shoulders.",
    ],
    &["You've got this! You'll hit Inbox Zero in no time."],
    &[
        "Nothing more satisfying than clearing out unneeded",
        "conversations.",
    ],
    &["Do you feel that? That's the feeling of progress."],
    &[
        "Less email in your inbox means more time to focus on",
        "what matters.",
    ],
    &["That must have felt incredible."],
    &[
        "Stronger, better, faster — and way fewer emails to ",
        "worry about.",
    ],
    &[
        "Why stop now? You're making space to focus on the",
        "right things.",
    ],
    &[
        "Poof! You've made emails disappear (but you can always",
        "find them later, of course.)",
    ],
    &["You're taking back your inbox."],
    &[
        "Impressive! You could make a mountain out of",
        "these emails.",
    ],
    &["Behold! You are the master of your domain (inbox)."],
    &["Nice! Those emails didn't spark joy."],
];

/// The send counts with a message of their own.
pub(super) const SEND_SPECIAL: &[(u64, &[&str])] = &[
    (25, &["To go even faster, use Snippets for common replies."]),
    (
        42,
        &[
            "The answer to life, the universe, and everything — ",
            "but mainly just your inbox.",
        ],
    ),
    (
        64,
        &[
            "Email 64. You've powered up your inbox, now blow on",
            "the cartridge and keep going!",
        ],
    ),
    (212, &["Your inbox is boiling. You're really cooking now!"]),
    (255, &["The highest level in Pac-Man. Waka waka."]),
    (
        314,
        &[
            "You can now roughly calculate the circumference",
            "of a circle.",
        ],
    ),
    (
        365,
        &["One for every day of the year! Happy email birthday?"],
    ),
    (
        404,
        &["But don't worry, we'll make sure that email _is_ found."],
    ),
    (455, &["Enough to reach the top of the Great Pyramid!"]),
    (666, &["Diabolically productive!"]),
    (
        777,
        &["Jackpot! If this was a slot machine, you'd be rich."],
    ),
    (1_000, &["Congrats! Welcome to the one comma club."]),
    (1_024, &["An entire kilobyte of email sends! Good work!"]),
    (1_123, &["This one's for Fibonacci."]),
    (1_234, &["Numerically satisfying! You're on a roll!"]),
    (1_337, &["w311 d0n3! Why did we stop talking like this?"]),
    (
        1_576,
        &[
            "A message for each step of the Empire State Building.",
            "How's the view?",
        ],
    ),
    (
        1_800,
        &[
            "No need for a toll-free number when you're this",
            "good at email.",
        ],
    ),
    (
        1_969,
        &[
            "One small step for email, one giant leap for your",
            "productivity.",
        ],
    ),
    (1_999, &["Incredible — this is exactly what Y2K felt like!"]),
    (
        2_500,
        &[
            "Your job isn't just sending emails, but if it was — ",
            "you deserve a raise.",
        ],
    ),
    (
        2_718,
        &[
            "The start of Euler's number. But rest assured —",
            "your emails are rational.",
        ],
    ),
    (
        3_142,
        &[
            "Nice, you can now find the circumference",
            "of a circle with better accuracy!",
        ],
    ),
    (
        3_405,
        &["You just completed the Tour de France. Marvelous!"],
    ),
    (3_840, &["The resolution of a 4k screen. Looking sharp!"]),
    (
        4_500,
        &[
            "Humans can see up to 4,500 stars in the night sky.",
            "And we think you're one of the brightest.",
        ],
    ),
    (
        4_877,
        &[
            "That's how much mail USPS processes each second.",
            "Good thing emails don't need stamps.",
        ],
    ),
    (5_000, &["Exo wants to be you when it grows up."]),
    (5_280, &["You've run a mile via email."]),
    (
        5_387,
        &[
            "The bottom of Lake Baikal! Is there any treasure down",
            "there?",
        ],
    ),
    (7_500, &["It's time to get your tires rotated."]),
    (9_001, &["That's over 9,000!"]),
    (10_000, &["What a milestone. Take a bow!"]),
    (12_345, &["You'll never forget how to count again!"]),
    (
        12_500,
        &["People are starting rumors that you broke a world record."],
    ),
    (
        15_000,
        &[
            "Unbelievable! Pinch yourself to make sure you're",
            "not dreaming.",
        ],
    ),
    (
        19_500,
        &[
            "Enough messages to sell out Madison Square Garden.",
            "Rock on.",
        ],
    ),
    (20_000, &["Quick, take a screenshot and share this online."]),
    (25_000, &["That's a whole lot of hellos and goodbyes."]),
    (30_000, &["You deserve a message in return: great job."]),
    (
        32_292,
        &[
            "You just tied Michael Jordan's career points — and you",
            "definitely beat his record for email.",
        ],
    ),
    (
        33_000,
        &["You're entering the stratosphere. Enjoy the view."],
    ),
    (
        40_000,
        &[
            "You've reached cruising altitude. You may now move",
            "about the cabin.",
        ],
    ),
    (45_000, &["The results are in: you are an email machine."]),
    (
        50_000,
        &["Take a moment to reflect on how far you've come."],
    ),
    (55_000, &["They will write songs about you."]),
    (60_000, &["Have you considered writing a book?"]),
    (65_000, &["You deserve a Wikipedia page."]),
    (
        70_000,
        &[
            "We're considering commissioning a sculpture in",
            "your honor.",
        ],
    ),
    (
        75_000,
        &[
            "Enough to contact everyone at Burning Man.",
            "If they were online.",
        ],
    ),
    (90_210, &["Welcome to the Beverly Hills of email."]),
    (100_000, &["An email legend! Is it lonely at the top?"]),
    (
        112_000,
        &[
            "The number of speeding tickets issued each day.",
            "But you get a pass, speedster.",
        ],
    ),
    (120_000, &["No one's ever gotten this far. Neat!"]),
    (
        125_000,
        &[
            "There's nothing else to say… we're hiring more",
            "copywriters.",
        ],
    ),
];

/// The Mark Done counts with a message of their own.
pub(super) const CLEAR_SPECIAL: &[(u64, &[&str])] = &[
    (50, &["Great work! You're gaining focus and clarity."]),
    (150, &["Gotta clear 'em all!"]),
    (
        260,
        &["Enough for the Monaco Grand Prix. Pop the champagne!"],
    ),
    (300, &["This. Is. Sparta…? No, just your inbox."]),
    (321, &["Go, go, go!"]),
    (451, &["At what temperature does an email burn?"]),
    (747, &["You're soaring through your inbox in first class."]),
    (1_111, &["Quick — make a wish!"]),
    (
        1_352,
        &["The equivalent of War and Peace — you're so cultured!"],
    ),
    (
        1_440,
        &[
            "A day's worth of minutes, a day's worth of Mark Done.",
            "Nice!",
        ],
    ),
    (
        1_543,
        &[
            "The year Copernicus published his heliocentric theory.",
            "What do you think he'd think about iPhones?",
        ],
    ),
    (1_618, &["Your inbox is looking golden."]),
    (
        1_800,
        &[
            "The estimated number of miles Frodo travelled to",
            "Mordor! The internet is an amazing place.",
        ],
    ),
    (
        1_984,
        &[
            "Big Brother's not watching. Your inbox is",
            "your own utopia.",
        ],
    ),
    (
        2_001,
        &["A Space Odyssey of email. HAL 9000 would be proud."],
    ),
    (
        2_222,
        &[
            "Deuces are wild! You've twinned your way to a",
            "cleaner inbox.",
        ],
    ),
    (2_600, &["New high score! Atari would be proud."]),
    (
        2_909,
        &["You just climbed the Burj Khalifa. Take a sip of water!"],
    ),
    (
        3_000,
        &["In the year 3,000 you'll be able to use Exo", "from space."],
    ),
    (3_600, &["One for every second in an hour."]),
    (
        4_047,
        &[
            "From 40.47 astronomical units away, Earth is a pale blue",
            "dot. But you're a shining star.",
        ],
    ),
    (5_555, &["Please high five your screen in celebration."]),
    (7_777, &["Today's your lucky day!"]),
    (8_080, &["Your server is cleaner than ever before."]),
    (10_000, &["Five digits strong! You're breathing rare air."]),
    (
        10_101,
        &[
            "That's 21 in binary. If you were a computer, you just",
            "hit blackjack.",
        ],
    ),
    (11_000, &["Your email amp goes to 11… thousand."]),
    (11_600, &["You could buy a house in 1960!"]),
    (14_000, &["Bravo! You deserve a 14k gold medal."]),
    (
        20_000,
        &[
            "Humans can't hear much beyond 20k Hz, but can you",
            "hear this? You're awesome.",
        ],
    ),
    (24_000, &["Incredible! Your inbox is as pure as 24k gold."]),
    (
        24_901,
        &["Enough to travel the circumference of the world."],
    ),
    (
        25_000,
        &[
            "Take notes, everyone! This is how you get",
            "through an inbox.",
        ],
    ),
    (
        27_182,
        &[
            "It's natural log! Let's calculate your inbox's",
            "radioactive decay.",
        ],
    ),
    (29_032, &["A Mount Everest of emails."]),
    (30_000, &["Unimportant emails are scared of you."]),
    (35_000, &["You shed email like a shark sheds teeth."]),
    (
        36_000,
        &["Welcome to the bottom of the Mariana Trench — of email."],
    ),
    (42_195, &["An entire marathon! Collect your medal."]),
    (
        50_000,
        &[
            "You're setting a positive example for all the young",
            "inboxes out there.",
        ],
    ),
    (
        54_321,
        &[
            "Blastoff! This feels even more special if you make rocket",
            "noises with your mouth.",
        ],
    ),
    (
        60_000,
        &[
            "Can you imagine if those were still in your inbox?",
            "_Shudder._",
        ],
    ),
    (75_000, &["Is this really happening?"]),
    (80_000, &["They'll make a Ted Talk about you one day."]),
    (
        89_214,
        &["Tom Brady's career yards. Welcome to the", "hall of fame."],
    ),
    (99_999, &["Amazing! Can our counter handle six digits?!"]),
    (100_000, &["Yes. Yes it can."]),
    (
        102_015,
        &["10% luck, 20% skill, 15% concentrated power of email."],
    ),
    (125_000, &["Haters will say it's fake."]),
    (
        148_157,
        &[
            "You've traveled as far as the Opportunity rover.",
            "Keep your battery charged!",
        ],
    ),
    (
        150_000,
        &["Historians should study you to see how you did it."],
    ),
    (175_000, &["There's only one word: wow."]),
    (186_282, &["You're clearing at the speed of light."]),
    (
        200_000,
        &["We didn't realize you could go this high. Neat!"],
    ),
    (
        209_117,
        &[
            "If you only read a word of each conversation, you",
            "finished Moby Dick. Ahoy matey.",
        ],
    ),
    (238_900, &["You've reached the moon. Is it made of cheese?"]),
    (
        250_000,
        &["And you wept, as you had no more emails to conquer."],
    ),
];
