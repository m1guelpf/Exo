//! What's on screen: the sign-in screen, the signed-out alert, or an account's mail, and
//! switching between accounts.
//!
//! On launch Exo opens every account still signed in: the one used last on screen (the first
//! still signed in, when Google signed that one out), and the others off it, where they sync,
//! send and notify all the same. Every grant is read from the
//! Keychain first, so its prompts come together. With no account the sign-in screen shows.
//! Switching puts another account's mail on screen, or the sign-in screen offering `Sign in as`
//! for one Google signed out; adding an account shows the sign-in screen with a way back. The
//! mail switched away from stays open off screen as it was left, and switching back picks it up.
//!
//! When Google refuses an open account's grant (at launch or later), it closes and stays listed
//! as signed out; on screen, its mail gives way to "Almost there…", which leads back to the
//! sign-in screen. A grant the Keychain won't hand over leaves its account closed, and "Almost
//! there…" shows when it would be on screen, without the listing. An account
//! whose store can't be opened shows a teardown saying why (`Sorry!`, [`Unopened`]) in
//! place of its mail, at launch or on switching to it, with the other accounts a ctrl+1…9 away;
//! Clear Cache and Restart makes its store again and opens it. After a
//! sign-in, Exo goes straight to the inbox: there's no first-run tutorial.
//! Signing out forgets the account and deletes what Exo keeps of it, then opens the next account
//! the way launch does.
//!
//! The window's tabs ([`tabs`]) each have a page of their own on an account: the screen is the
//! active tab's, and showing another tab puts its page up as it was left, or a new page at its
//! route. Switching accounts happens in the tab on screen; the page switched away from stays
//! open off screen as it was left, at most one an account.

use std::path::{Path, PathBuf};

use exo_auth::Tokens;

use crate::{
    Clock, Mail,
    account::{Account, OpenError, Presence},
    accounts::Request,
    chrome,
    desktop::{Desktop, QuitWarning},
    log_in::{self, Back, LogIn},
    notifications::{Click, Notifier},
    profile_pictures::ProfilePictures,
    recovery::{self, Frames},
    sign_in::{MailError, SignIn},
    signed_out::SignedOut,
    tabs::{self, Parked, Route, Tabs},
    teardown::{Choice, Reason, Unopened},
    zoom::Zoom,
};

/// What Exo starts with.
pub enum Start {
    /// This account's mail, signed in by the caller (screenshots and tests on gmail-mock).
    Mail(Box<Account>),
    /// Whatever sign-in says: the last used account's mail, or the sign-in screen.
    SignIn(SignIn),
}

/// The screen showing.
pub enum Screen {
    /// The sign-in screen.
    LogIn(LogIn),
    /// "Almost there…": Google signed the account out.
    SignedOut(SignedOut),
    /// "Sorry!": the account's store couldn't be opened.
    Unopened(Unopened),
    /// An account's mail.
    Mail(Box<Mail>),
}

/// Why Exo couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// A system font Exo draws with is missing.
    #[error(transparent)]
    Assets(#[from] exo_ui::AssetError),
    /// The account list can't be read.
    #[error(transparent)]
    Accounts(#[from] exo_auth::Error),
    /// The first account's grant can't be read.
    #[error(transparent)]
    Mail(#[from] MailError),
}

/// The app: the screen showing, the accounts open off it, and how to sign in.
pub struct App {
    screen: Screen,
    /// `None` when the caller opened the account itself.
    sign_in: Option<SignIn>,
    /// The grants of the accounts sign-in opened, which say when Google refuses one.
    grants: Vec<(String, Tokens)>,
    /// The accounts' pages no tab holds, at most one an account.
    background: Vec<Away>,
    assets: PathBuf,
    /// What dates are relative to.
    clock: Clock,
    /// The window as a Mac app's, when Exo runs as one ([`App::set_desktop`]).
    desktop: Option<Desktop>,
    /// Where the accounts post their notifications, when Exo can ([`App::set_notifier`]).
    notifier: Option<Notifier>,
    /// The frames, which survive a panic (`recovery`).
    frames: Frames,
    /// `Warn Before Quitting`, for the settings menu ([`App::set_quit_warning`]).
    quit_warning: Option<QuitWarning>,
    /// The accounts' profile photos, for the switchers.
    pictures: ProfilePictures,
    /// The app's zoom ([`App::set_zoom`]).
    zoom: Zoom,
    /// The window's tabs: the screen is the active one's.
    tabs: Tabs,
    /// Where the tabs are kept ([`App::keep_tabs`]); none for tabs that last the run.
    tabs_file: Option<tabs::File>,
    /// The day See Your Week was on when it was switched away from, for the next account's mail
    /// to open on.
    kept_week: Option<jiff::civil::Date>,
    /// The updater, for the mail on screen's alerts ([`App::set_updates`]).
    updates: Option<crate::update::Updates>,
    /// What the update that relaunched Exo waits for: word that the window is up.
    receipt: Option<fastframe_update::Receipt>,
}

/// An account's page that no tab holds. Its account syncs, sends and notifies
/// as the one on screen does, at the pace of an account no window shows
/// ([`Presence::OffScreen`]) unless a tab shows it.
enum Away {
    /// Opened at launch, and not on screen since: its page is made when it first shows.
    Account(Box<Account>),
    /// A page, as it was left: switched away from in its tab, or its account's last when its
    /// tab closed.
    Mail(Box<Mail>),
}

impl Away {
    /// `account`, opened off screen, its reminders dated by `clock`.
    fn opened(account: Account, clock: &Clock) -> Self {
        account.set_on_screen(false);
        account.set_presence(Presence::OffScreen);
        account.set_clock(clock.clone());
        Self::Account(Box::new(account))
    }

    /// The account, whether its page has been made yet or not.
    fn account(&self) -> &Account {
        match self {
            Self::Account(account) => account,
            Self::Mail(mail) => mail.account(),
        }
    }

    /// The account, to change.
    fn account_mut(&mut self) -> &mut Account {
        match self {
            Self::Account(account) => account,
            Self::Mail(mail) => mail.account_mut(),
        }
    }

    /// The page: this one, or the first of an account opened at launch.
    fn into_page(self, ctx: &egui::Context, assets: &Path, clock: &Clock) -> Box<Mail> {
        match self {
            Self::Account(account) => Box::new(Mail::new(ctx, assets, *account, clock.clone())),
            Self::Mail(mail) => mail,
        }
    }
}

impl App {
    /// Sets up `cc`'s context to draw Exo, with the images in `assets`, and shows what
    /// `start` says, dated relative to `clock`. Reads the Keychain for every account's grant.
    ///
    /// # Errors
    /// When a system font can't be loaded, the account list can't be read, or the first account's
    /// mail can't be opened.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        assets: &Path,
        start: Start,
        clock: Clock,
    ) -> Result<Self, LaunchError> {
        let ctx = &cc.egui_ctx;
        exo_ui::install(ctx)?;
        let (launch, sign_in) = match start {
            Start::Mail(account) => (
                Launch::screen(Screen::Mail(Box::new(Mail::new(
                    ctx,
                    assets,
                    *account,
                    clock.clone(),
                )))),
                None,
            ),
            Start::SignIn(mut sign_in) => {
                (launch(&mut sign_in, ctx, assets, &clock)?, Some(sign_in))
            }
        };
        let mut app = Self {
            screen: launch.screen,
            sign_in,
            grants: launch.grants,
            background: launch.background,
            assets: assets.to_owned(),
            clock,
            desktop: None,
            notifier: None,
            frames: Frames::default(),
            quit_warning: None,
            pictures: ProfilePictures::new(ctx),
            zoom: Zoom::default(),
            tabs: Tabs::default(),
            tabs_file: None,
            kept_week: None,
            updates: None,
            receipt: None,
        };
        Zoom::install(ctx);
        app.share_accounts();
        app.refresh_picture();
        Ok(app)
    }

    /// Runs as a Mac app ([`Desktop`]): closing the window hides it while Exo keeps syncing,
    /// and the Dock badge counts the list on screen. Without, closing the window quits.
    pub fn set_desktop(&mut self, desktop: Desktop) {
        self.desktop = Some(desktop);
    }

    /// Posts the accounts' notifications with `notifier`: the mail on screen's, and every
    /// account's open off screen. Without, Exo shows none.
    pub fn set_notifier(&mut self, notifier: Notifier) {
        self.notifier = Some(notifier);
        self.share_notifier();
    }

    /// Shows `warning` (`Warn Before Quitting`, Exo's own) in the settings menu. Without, the
    /// menu has no such row.
    pub fn set_quit_warning(&mut self, warning: QuitWarning) {
        if let Screen::Mail(mail) = &mut self.screen {
            mail.set_quit_warning(Some(warning.clone()));
        }
        self.quit_warning = Some(warning);
    }

    /// Checks for updates with `updates`: ⌘K's `Check for Updates` and its alerts, in the mail on
    /// screen. Without, ⌘K has none.
    pub fn set_updates(&mut self, updates: crate::update::Updates) {
        if let Screen::Mail(mail) = &mut self.screen {
            mail.set_updates(Some(updates.clone()));
        }
        self.updates = Some(updates);
    }

    /// Exo was relaunched to finish installing an update: once its window has drawn, `receipt`
    /// tells the update's helper, which otherwise puts the previous Exo back.
    pub fn set_update_receipt(&mut self, receipt: fastframe_update::Receipt) {
        self.receipt = Some(receipt);
    }

    /// Zooms as `zoom` has it, which keeps what it's zoomed to. Without, the zoom lasts the run.
    pub fn set_zoom(&mut self, zoom: Zoom) {
        self.zoom = zoom;
    }

    /// Keeps the window's tabs in `dir`, the state folder, from now on, and puts back those kept
    /// there, the active one on screen at its route: the page launch opened, when it's on that
    /// tab's account. Without, the tabs last the run.
    pub fn keep_tabs(&mut self, ctx: &egui::Context, dir: &Path) {
        let (file, restored) = tabs::File::load(dir);
        self.tabs_file = Some(file);
        let Some((tabs, active)) = restored else {
            return;
        };
        self.tabs = tabs;
        match active {
            Parked::Route(route) if self.current() == Some(route.email.as_str()) => {
                if let Some(mail) = self.mail_mut() {
                    mail.go_to_route(&route);
                }
            }
            active => {
                if let Screen::Mail(page) = self.bring(ctx, active) {
                    self.leave(page);
                }
            }
        }
    }

    /// How many tabs the window has.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Which tab is on screen, from 0.
    pub fn active_tab(&self) -> usize {
        self.tabs.active()
    }

    /// What each tab's pill says, in order.
    pub fn tab_titles(&self) -> Vec<String> {
        self.tabs
            .pills()
            .into_iter()
            .map(|pill| pill.title)
            .collect()
    }

    /// Gives every open account the notifier, when there is one.
    fn share_notifier(&self) {
        let Some(notifier) = &self.notifier else {
            return;
        };
        for account in self.handles() {
            account.set_notifier(Some(notifier.clone()));
        }
    }

    /// Every handle on an account open: the page on screen's, the tabs', and those off screen.
    fn handles(&self) -> impl Iterator<Item = &Account> {
        self.mail()
            .map(Mail::account)
            .into_iter()
            .chain(self.tabs.pages().map(Mail::account))
            .chain(self.background.iter().map(Away::account))
    }

    /// A handle on `email`'s account, if it's open.
    fn account_of(&self, email: &str) -> Option<&Account> {
        self.handles().find(|account| account.email() == email)
    }

    /// A page on `email`'s account: the one on screen, a tab's, or the one off screen.
    fn page_of(&self, email: &str) -> Option<&Mail> {
        let off_screen = self.background.iter().filter_map(|away| match away {
            Away::Mail(page) => Some(&**page),
            Away::Account(_) => None,
        });
        self.mail()
            .into_iter()
            .chain(self.tabs.pages())
            .chain(off_screen)
            .find(|page| page.account().email() == email)
    }

    /// The screen showing.
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// The account's mail, if that's what's showing.
    pub fn mail(&self) -> Option<&Mail> {
        match &self.screen {
            Screen::Mail(mail) => Some(mail),
            Screen::LogIn(_) | Screen::SignedOut(_) | Screen::Unopened(_) => None,
        }
    }

    /// The account's mail, if that's what's showing, to change.
    pub fn mail_mut(&mut self) -> Option<&mut Mail> {
        match &mut self.screen {
            Screen::Mail(mail) => Some(mail),
            Screen::LogIn(_) | Screen::SignedOut(_) | Screen::Unopened(_) => None,
        }
    }

    /// The accounts open off screen: those of the tabs not on screen and of the pages no tab
    /// holds, each once, but the account on screen.
    pub fn off_screen(&self) -> impl Iterator<Item = &Account> {
        let mut seen = self.current().into_iter().collect::<Vec<_>>();
        self.tabs
            .pages()
            .map(Mail::account)
            .chain(self.background.iter().map(Away::account))
            .filter(move |account| {
                let email = account.email();
                let new = !seen.contains(&email);
                seen.push(email);
                new
            })
    }

    /// Nothing is in flight on the screen showing, nor in the accounts open off it. For
    /// screenshots and tests.
    pub fn is_idle(&self) -> bool {
        let screen = match &self.screen {
            Screen::Mail(mail) => mail.is_idle(),
            Screen::LogIn(log_in) => log_in.is_idle(),
            Screen::SignedOut(alert) => alert.is_idle(),
            Screen::Unopened(alert) => alert.is_idle(),
        };
        screen && self.off_screen().all(Account::is_idle) && self.pictures.is_idle()
    }

    /// Draws one frame and handles its input.
    pub fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        // The shell's zoom keys, before the page sees them.
        self.zoom.frame(&ctx);
        // The shell's tab keys, whatever has the keyboard.
        for command in tabs::keys(&ctx, self.tabs.len() > 1) {
            self.tab_command(&ctx, command);
        }
        // Find's keys before anything else reads the keyboard; there's none on the sign-in screen.
        crate::find::begin(&ctx, matches!(self.screen, Screen::Mail(_)));
        self.refresh_pills();
        chrome::set_tabs(&ctx, self.tabs.pills(), self.tabs.active());
        // Text the Mac's drag has over the window, and a selection dragged out of it.
        crate::desktop::drag::begin(&ctx);
        self.screen_frame(ui);
        crate::desktop::drag::end(&ctx);
        // What the screen's strip showed and what was clicked in it.
        let (action, shown) = chrome::take(&ctx);
        if let Some(pill) = shown {
            self.tabs.shown(pill);
        }
        match action {
            Some(chrome::Action::Select(index)) => self.activate(&ctx, index),
            Some(chrome::Action::Close(index)) => self.close_tab(&ctx, index),
            Some(chrome::Action::NewTab) => self.new_tab(&ctx),
            Some(chrome::Action::CopyLink(index)) => self.copy_link(&ctx, index),
            None => {}
        }
    }

    /// The screen's part of a frame: what's on screen draws, and what it asks for is done.
    fn screen_frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.desktop.as_mut().is_some_and(Desktop::take_clear_cache) {
            self.clear_cache_and_restart(&ctx);
        }
        if self.desktop.as_mut().is_some_and(Desktop::take_reload) {
            self.reload(&ctx);
        }
        // Edit ▸ `Enable Autocorrect`: every account, in every tab, the other way from the one on
        // screen.
        if self
            .desktop
            .as_mut()
            .is_some_and(Desktop::take_toggle_autocorrect)
            && let Some(on) = self.mail().map(|mail| !mail.autocorrect())
        {
            let off_screen = self
                .background
                .iter_mut()
                .map(Away::account_mut)
                .chain(self.tabs.pages_mut().map(Mail::account_mut));
            for account in off_screen {
                account.update_settings(|settings| settings.typing_assistance.enabled = Some(on));
            }
            if let Some(mail) = self.mail_mut() {
                mail.set_autocorrect(on);
            }
        }
        if let Some(desktop) = &self.desktop {
            desktop.set_autocorrect(self.mail().map(Mail::autocorrect));
        }
        self.close_refused(&ctx);
        // Links another launch or macOS handed over: a new message to a `mailto:`
        // one, in a new tab; one of Exo's in the tab for the account it names. They wait while no
        // account's mail shows.
        let links = match (&self.screen, &mut self.desktop) {
            (Screen::Mail(_), Some(desktop)) => desktop.take_links(),
            _ => Vec::new(),
        };
        for url in links {
            match crate::links::parse(&url) {
                Some(link) => self.open_link(&ctx, &link, false),
                // A `mailto:` link: a new message in a new tab, on the account used last, the
                // active tab's.
                None => {
                    self.new_tab(&ctx);
                    if let Screen::Mail(mail) = &mut self.screen {
                        mail.compose_mailto(&url, false);
                    }
                }
            }
        }
        let clicks = self
            .desktop
            .as_mut()
            .map(Desktop::take_clicks)
            .unwrap_or_default();
        for click in clicks {
            self.clicked(&ctx, click);
        }
        // Its store stopped working: a teardown saying why, in place of its mail. Its
        // pages close, and the account with them (sending what's held, as quitting does); the
        // tabs that showed it keep their routes.
        if let Screen::Mail(mail) = &self.screen
            && let Some(unusable) = mail.account().unusable()
        {
            let email = mail.account().email().to_owned();
            let alert = Unopened::new(&ctx, &self.clock.now(), email.clone(), unusable.into());
            drop(self.close_pages(&email));
            self.screen = Screen::Unopened(alert);
            self.grants.retain(|(open, _)| *open != email);
        }
        match &mut self.screen {
            Screen::Mail(mail) => {
                if let Some(request) = mail.frame(ui) {
                    self.request(&ctx, request);
                }
            }
            Screen::SignedOut(alert) => {
                if let Some(log_in) = alert.show(ui) {
                    // The alert reloads to `/new`: nothing was on screen before, but another
                    // account still signed in is somewhere to go back to.
                    let signed_out = alert.email().to_owned();
                    let back = self.back(None, Some(&signed_out));
                    self.screen = Screen::LogIn(log_in.with_back(back));
                }
            }
            Screen::Unopened(alert) => {
                let choice = alert.show(ui);
                let email = alert.email().to_owned();
                match choice {
                    Some(Choice::Reload) => self.switch(&ctx, &email),
                    Some(Choice::ClearCache) => self.clear_cache(&ctx, &email),
                    Some(Choice::Switch(place)) => {
                        if let Some(account) = self.listed().into_iter().nth(place) {
                            self.switch(&ctx, &account.email);
                        }
                    }
                    None => {}
                }
            }
            Screen::LogIn(log_in) => {
                // The sign-in screen only shows when there's sign-in.
                let Some(sign_in) = &mut self.sign_in else {
                    return;
                };
                match log_in.show(ui, sign_in) {
                    Some(log_in::Outcome::SignedIn(email)) => self.signed_in(&ctx, &email),
                    Some(log_in::Outcome::Back(email)) => self.switch(&ctx, &email),
                    None => {}
                }
            }
        }
    }

    /// Carries out what the mail on screen asked for.
    fn request(&mut self, ctx: &egui::Context, request: Request) {
        match request {
            Request::Open(link) => self.open_link(ctx, &link, true),
            _ if self.sign_in.is_none() => log::warn!(
                "switching or signing accounts out needs Exo's sign-in, which this app wasn't started with"
            ),
            Request::SignOut(email) => self.sign_out(ctx, &email),
            Request::CheckSignOut(email) => self.check_sign_out(ctx, &email),
            Request::Switch(email) => {
                self.keep_week();
                self.switch(ctx, &email);
            }
            Request::Add => {
                self.keep_week();
                // Adding an account: the sign-in screen, with a way back to this one.
                let back = self.current().map(|email| Back::Previous(email.to_owned()));
                let log_in = self.log_in_screen(ctx).with_back(back);
                drop(self.show(Screen::LogIn(log_in)));
            }
            Request::Move { email, to } => self.change_list(|auth| auth.move_account(&email, to)),
            Request::Color { email, color } => {
                self.change_list(|auth| auth.set_color(&email, Some(color.to_owned())))
            }
        }
    }

    /// Opens one of Exo's links on the account it names, else the account on screen's: one macOS
    /// or another launch handed over in that account's tab, else the tab on screen; one clicked
    /// in a message, `in_new_tab`, in a new tab.
    fn open_link(&mut self, ctx: &egui::Context, link: &crate::links::Link, in_new_tab: bool) {
        match (&link.email, in_new_tab) {
            (Some(email), true) => self.open_tab(ctx, Parked::Route(Route::home(email.clone()))),
            (None, true) => self.new_tab(ctx),
            (Some(email), false) => self.tab_for(ctx, email),
            (None, false) => {}
        }
        if let Screen::Mail(mail) = &mut self.screen {
            mail.open_link(link);
        }
    }

    /// The strip's copy button: tab `index`'s link on the clipboard, as
    /// `text/plain` and `text/html`, and the tab on screen saying so. A tab on the sign-in
    /// screen or an alert has none.
    fn copy_link(&mut self, ctx: &egui::Context, index: usize) {
        let url = if index == self.tabs.active() {
            self.mail().map(Mail::link)
        } else {
            self.tabs.route(index).and_then(|route| {
                let account = self.account_of(&route.email)?;
                Some(crate::links::url(
                    &route,
                    &account.snapshot().labels,
                    account.settings(),
                ))
            })
        };
        let Some(url) = url else { return };
        crate::clipboard::write(ctx, &url, &url);
        if let Some(mail) = self.mail_mut() {
            mail.link_copied();
        }
    }

    /// Switching accounts with See Your Week up: the next account's mail opens on its week.
    fn keep_week(&mut self) {
        self.kept_week = self.mail().and_then(Mail::calendar_week_to_keep);
    }

    /// Google refused an open account's grant: its pages close, and it's listed as signed out,
    /// so switching to it offers to sign it in again; on screen, its mail gives way to "Almost
    /// there…". What it queued stays in its store's outbox for the next sign-in.
    fn close_refused(&mut self, ctx: &egui::Context) {
        let refused: Vec<String> = self
            .grants
            .iter()
            .filter(|(_, tokens)| tokens.needs_sign_in())
            .map(|(email, _)| email.clone())
            .collect();
        if refused.is_empty() {
            return;
        }
        for email in refused {
            log::warn!("an account was signed out of Google");
            self.grants.retain(|(open, _)| *open != email);
            drop(self.close_pages(&email));
            if self.current() == Some(email.as_str()) {
                self.screen = Screen::SignedOut(SignedOut::new(
                    ctx,
                    &self.assets.join("images"),
                    &self.clock.now(),
                    email.clone(),
                ));
            }
            if let Some(sign_in) = &mut self.sign_in
                && let Err(error) = sign_in.auth().and_then(|auth| auth.mark_signed_out(&email))
            {
                log::error!("listing an account as signed out: {error}");
            }
        }
        self.share_accounts();
    }

    /// Takes every page of `email`'s account but the one on screen out of the tabs (which keep
    /// their routes) and off screen, for the account to close.
    fn close_pages(&mut self, email: &str) -> Vec<Away> {
        let mut pages: Vec<Away> = self
            .tabs
            .unpage(email)
            .into_iter()
            .map(Away::Mail)
            .collect();
        let (gone, kept) = std::mem::take(&mut self.background)
            .into_iter()
            .partition::<Vec<_>, _>(|away| away.account().email() == email);
        self.background = kept;
        pages.extend(gone);
        pages
    }

    /// Opens what a clicked notification is about, in its account's tab, so another account's
    /// mail switches to it first.
    fn clicked(&mut self, ctx: &egui::Context, click: Click) {
        self.tab_for(ctx, &click.account);
        match &mut self.screen {
            Screen::Mail(mail) if mail.account().email() == click.account => {
                mail.open_notification(click.open)
            }
            _ => log::info!("a notification's account isn't open"),
        }
    }

    /// Shows `email` in the tab on screen: its mail, or the sign-in screen
    /// offering it when Google signed it out, with a way back to the account on screen. The page
    /// it replaces stays open off screen, as it was left.
    fn switch(&mut self, ctx: &egui::Context, email: &str) {
        if let Some(screen) = self.screen_for(ctx, email, true) {
            drop(self.show(screen));
        }
    }

    /// What showing `email` puts up: a page of its mail ([`page_for`](Self::page_for), picking
    /// up the one off screen when `pick_up`), or the sign-in screen offering it when Google
    /// signed it out, with a way back to the account on screen, "Almost there…" when the
    /// Keychain won't hand its grant over, "Sorry!" when its store can't be opened. None when it
    /// isn't on this Mac or can't be opened otherwise.
    fn screen_for(&mut self, ctx: &egui::Context, email: &str, pick_up: bool) -> Option<Screen> {
        if self.sign_in.is_some() {
            let account = self
                .listed()
                .into_iter()
                .find(|account| account.email == email)?;
            if account.signed_out {
                let previous = self.current().map(str::to_owned);
                let back = self.back(previous.as_deref(), None);
                return Some(Screen::LogIn(
                    self.log_in_screen(ctx)
                        .with_hint(account.email)
                        .with_back(back),
                ));
            }
        }
        match self.page_for(ctx, email, pick_up) {
            Ok(page) => Some(Screen::Mail(page)),
            // As at launch: the Keychain won't hand the grant over.
            Err(MailError::Auth(error @ exo_auth::Error::Secrets(_))) => {
                log::warn!("reading an account's grant: {error}");
                Some(Screen::SignedOut(SignedOut::new(
                    ctx,
                    &self.assets.join("images"),
                    &self.clock.now(),
                    email.to_owned(),
                )))
            }
            Err(MailError::Open(error)) => {
                log::error!("opening an account: {error}");
                Some(Screen::Unopened(Unopened::new(
                    ctx,
                    &self.clock.now(),
                    email.to_owned(),
                    Reason::of(&error),
                )))
            }
            Err(error) => {
                log::error!("opening an account: {error}");
                None
            }
        }
    }

    /// `email`'s store couldn't be opened: a teardown saying why, in place of its mail.
    /// Returns the screen it replaced, as [`show`](Self::show) does.
    fn unopened(&mut self, ctx: &egui::Context, email: &str, error: &OpenError) -> Option<Screen> {
        log::error!("opening an account: {error}");
        let alert = Unopened::new(ctx, &self.clock.now(), email.to_owned(), Reason::of(error));
        self.show(Screen::Unopened(alert))
    }

    /// Clear Cache and Restart for `email`, whose store couldn't be opened: its store is made
    /// again, keeping what Gmail doesn't have yet, and its attachment previews go; then its mail
    /// opens, to fill from Gmail. Only this account's store is broken, so the others stay.
    fn clear_cache(&mut self, ctx: &egui::Context, email: &str) {
        let Some(sign_in) = &self.sign_in else { return };
        if let Err(error) = sign_in.clear_cache(email) {
            drop(self.unopened(ctx, email, &OpenError::Store(error)));
            return;
        }
        remove_previews(email);
        self.switch(ctx, email);
    }

    /// The Help menu's Clear Cache and Restart, once confirmed: every account's mail closes
    /// (sending what's held for Undo, as quitting does), its store is made again, keeping what
    /// Gmail doesn't have yet, and Exo opens what launch would.
    pub fn clear_cache_and_restart(&mut self, ctx: &egui::Context) {
        if self.sign_in.is_none() {
            return log::warn!(
                "clearing the cache needs Exo's sign-in, which this app wasn't started with"
            );
        }
        if let Screen::Mail(mail) = &mut self.screen {
            mail.save_drafts();
        }
        let placeholder = Screen::LogIn(self.log_in_screen(ctx));
        drop(std::mem::replace(&mut self.screen, placeholder));
        self.background.clear();
        self.grants.clear();
        // It starts over as a relaunch would, with one tab; the others' pages close.
        self.tabs = Tabs::default();
        let listed = self.listed();
        let Some(sign_in) = &mut self.sign_in else {
            return;
        };
        for account in listed {
            match sign_in.clear_cache(&account.email) {
                Ok(()) => remove_previews(&account.email),
                Err(error) => log::error!("clearing an account's cache: {error}"),
            }
        }
        match launch(sign_in, ctx, &self.assets, &self.clock) {
            Ok(launch) => {
                (self.screen, self.grants, self.background) =
                    (launch.screen, launch.grants, launch.background)
            }
            Err(error) => log::error!("opening the first account: {error}"),
        }
        self.share_notifier();
        self.settle_on_screen();
    }

    /// View ▸ `Reload`: what's on screen starts over where it is. The page on screen closes,
    /// saving its drafts, and a new one opens at the list and conversation it showed (the page's
    /// URL); its account closes, sending what it holds as quitting does, and opens again from
    /// its store only when no other tab has it. An alert in its place tries opening the account
    /// again; the sign-in screen stays.
    fn reload(&mut self, ctx: &egui::Context) {
        if self.sign_in.is_none() {
            return log::warn!("reloading needs Exo's sign-in, which this app wasn't started with");
        }
        let route = match &self.screen {
            Screen::Mail(mail) => Route {
                view: tabs::View::List,
                ..mail.route()
            },
            Screen::SignedOut(alert) => Route::home(alert.email().to_owned()),
            Screen::Unopened(alert) => Route::home(alert.email().to_owned()),
            Screen::LogIn(_) => return,
        };
        let placeholder = Screen::LogIn(self.log_in_screen(ctx));
        drop(std::mem::replace(&mut self.screen, placeholder));
        drop(self.go(ctx, route));
    }

    /// An account just signed in on the sign-in screen: its mail, welcoming it the first time it
    /// opens on this Mac.
    fn signed_in(&mut self, ctx: &egui::Context, email: &str) {
        let first_time = self
            .sign_in
            .as_ref()
            .is_some_and(|sign_in| !sign_in.has_store(email));
        match self.page_for(ctx, email, true) {
            Ok(mut mail) => {
                if first_time {
                    mail.welcome_first_time();
                }
                drop(self.show(Screen::Mail(mail)));
            }
            Err(MailError::Open(error)) => drop(self.unopened(ctx, email, &error)),
            Err(error) => {
                log::error!("opening an account: {error}");
                if let Screen::LogIn(log_in) = &mut self.screen {
                    log_in.fail();
                }
            }
        }
    }

    /// A page on `email`'s mail: the one off screen when `pick_up` (a tab switching to the
    /// account picks up the page it left there) or when it's an account opened at launch and
    /// not shown yet; else a new page beside one of its pages; else the account, opened now.
    fn page_for(
        &mut self,
        ctx: &egui::Context,
        email: &str,
        pick_up: bool,
    ) -> Result<Box<Mail>, MailError> {
        let off_screen = self.background.iter().position(|away| {
            away.account().email() == email && (pick_up || matches!(away, Away::Account(_)))
        });
        if let Some(index) = off_screen {
            return Ok(self.background.swap_remove(index).into_page(
                ctx,
                &self.assets,
                &self.clock,
            ));
        }
        if let Some(page) = self.page_of(email) {
            return Ok(page.another_page(&self.assets));
        }
        let sign_in = self.sign_in.as_mut().expect("only sign-in opens accounts");
        let (account, tokens) = sign_in.open_mail(email, ctx)?;
        // In place of the grant it had when it was open before (View ▸ Reload).
        self.grants.retain(|(open, _)| open != email);
        self.grants.push((email.to_owned(), tokens));
        Ok(Box::new(Mail::new(
            ctx,
            &self.assets,
            account,
            self.clock.clone(),
        )))
    }

    /// Puts `screen` up. A page it replaces stays open off screen, as it was left; mail it shows
    /// becomes the account used last. Returns what it replaced when that wasn't a page.
    fn show(&mut self, screen: Screen) -> Option<Screen> {
        match self.put(screen) {
            Screen::Mail(page) => {
                self.leave(page);
                None
            }
            screen => Some(screen),
        }
    }

    /// Puts `screen` up and returns what it replaced, a page leaving the screen as it goes (its
    /// drafts saved, and sent to Gmail now).
    fn put(&mut self, screen: Screen) -> Screen {
        let mut previous = std::mem::replace(&mut self.screen, screen);
        if let Screen::Mail(page) = &mut previous {
            page.leave_screen();
        }
        self.settle_on_screen();
        previous
    }

    /// `page`, which no tab holds now, stays open off screen as it was left, in place of the
    /// page of its account there before.
    fn leave(&mut self, page: Box<Mail>) {
        self.background
            .retain(|away| away.account().email() != page.account().email());
        self.background.push(Away::Mail(page));
    }

    /// The page of a tab that closed goes, saving its drafts, unless it's its account's last,
    /// which stays open off screen as it was left: the account keeps syncing and notifying.
    fn closed(&mut self, page: Box<Mail>) {
        let email = page.account().email();
        if self.account_of(email).is_none() {
            self.background.push(Away::Mail(page));
        }
    }

    /// The page on screen (if that's what shows) asks for frames, takes up its account's latest
    /// settings, becomes the account used last, posts its notifications, learns the account list,
    /// `Warn Before Quitting` and the updater, and has its profile photo asked for.
    fn settle_on_screen(&mut self) {
        if let Screen::Mail(mail) = &mut self.screen {
            mail.account_mut().catch_up();
            mail.account().set_on_screen(true);
            mail.set_quit_warning(self.quit_warning.clone());
            mail.set_updates(self.updates.clone());
            if let Some(day) = self.kept_week.take() {
                mail.calendar_keep_week(day);
            }
        }
        if let (Screen::Mail(mail), Some(notifier)) = (&self.screen, &self.notifier) {
            mail.account().set_notifier(Some(notifier.clone()));
        }
        if let (Screen::Mail(mail), Some(sign_in)) = (&self.screen, &mut self.sign_in)
            && let Err(error) = sign_in
                .auth()
                .and_then(|auth| auth.use_account(mail.account().email()))
        {
            log::error!("remembering the account used last: {error}");
        }
        self.share_accounts();
        self.refresh_picture();
    }

    /// Signs `email` out: forgets its grant, stops it
    /// without sending what it holds (the user was asked), and deletes its store and attachment
    /// previews. Signing out of the account on screen shows the next one as launch picks it (open
    /// off screen, or opened now), or the sign-in screen.
    fn sign_out(&mut self, ctx: &egui::Context, email: &str) {
        let Some(sign_in) = &mut self.sign_in else {
            return;
        };
        // Nothing is touched when the grant can't be forgotten: the account stays signed in.
        if let Err(error) = sign_in.forget(email) {
            log::error!("signing an account out: {error}");
            return;
        }
        if let Ok(auth) = sign_in.auth() {
            self.pictures.forget(auth, email);
        }
        self.grants.retain(|(open, _)| open != email);
        // Its pages close without sending what it holds: one handle stops it for every page.
        let on_screen = self.current() == Some(email);
        let mut pages = self.close_pages(email);
        let previews = self
            .page_of(email)
            .map(|page| page.preview_cache().to_owned())
            .or_else(|| {
                pages.iter().find_map(|away| match away {
                    Away::Mail(page) => Some(page.preview_cache().to_owned()),
                    Away::Account(_) => None,
                })
            });
        let previews = previews.unwrap_or_else(|| crate::attachment_files::default_cache(email));
        match (self.mail_mut().filter(|_| on_screen), pages.first_mut()) {
            (Some(mail), _) => mail.discard(),
            (None, Some(away)) => away.account_mut().discard(),
            (None, None) => {}
        }
        drop(pages);
        if on_screen {
            // The signed-out account's mail closes here, and its store with it.
            self.screen = Screen::LogIn(self.log_in_screen(ctx));
            if let Some(next) = self
                .sign_in
                .as_mut()
                .and_then(|sign_in| sign_in.auth().ok())
                .and_then(exo_auth::Auth::default_account)
            {
                self.switch(ctx, &next.email);
            }
        }
        if let Some(sign_in) = &self.sign_in {
            sign_in.delete_store(email);
        }
        for cache in std::iter::once(previews).chain(crate::account::parts_cache(email)) {
            if let Err(error) = std::fs::remove_dir_all(&cache)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                log::error!("deleting an account's attachment cache: {error}");
            }
        }
        self.settle_on_screen();
    }

    /// Signs `email`, not the account on screen, out: at once when it has nothing unsent, else
    /// after the mail on screen asks (`One last thing…`). Its store says what it hasn't sent,
    /// the calendar's changes included. A store that can't be opened holds nothing Exo can
    /// send, and goes without asking; otherwise nothing happens when that can't be counted.
    fn check_sign_out(&mut self, ctx: &egui::Context, email: &str) {
        let Some(sign_in) = &mut self.sign_in else {
            return;
        };
        let unsent = match sign_in.unsent(email) {
            Ok(unsent) => unsent,
            Err(MailError::Open(OpenError::Store(error))) if error.unusable().is_some() => {
                log::warn!("signing an account out without counting what it hasn't sent: {error}");
                exo_store::Unsent::default()
            }
            Err(error) => return log::error!("counting what an account hasn't sent: {error}"),
        };
        let asked = match &mut self.screen {
            Screen::Mail(mail) => mail.confirm_sign_out(email.to_owned(), unsent),
            Screen::LogIn(_) | Screen::SignedOut(_) | Screen::Unopened(_) => false,
        };
        if !asked {
            self.sign_out(ctx, email);
        }
    }

    /// Changes the account list with `change`, and tells the mail on screen.
    fn change_list(&mut self, change: impl FnOnce(&exo_auth::Auth) -> Result<(), exo_auth::Error>) {
        if let Some(sign_in) = &mut self.sign_in
            && let Err(error) = sign_in.auth().and_then(change)
        {
            log::error!("changing the account list: {error}");
        }
        self.share_accounts();
    }

    /// The address of the mail on screen.
    fn current(&self) -> Option<&str> {
        self.mail().map(|mail| mail.account().email())
    }

    /// The account list, empty when there's no sign-in or it can't be read.
    fn listed(&mut self) -> Vec<exo_auth::Account> {
        let Some(sign_in) = &mut self.sign_in else {
            return Vec::new();
        };
        sign_in
            .auth()
            .map(|auth| auth.accounts())
            .unwrap_or_default()
    }

    /// Where the sign-in screen's back button leads: to `previous`, the
    /// account it was opened from, else to another account still signed in than `except`.
    fn back(&mut self, previous: Option<&str>, except: Option<&str>) -> Option<Back> {
        if let Some(previous) = previous {
            return Some(Back::Previous(previous.to_owned()));
        }
        let other = self
            .listed()
            .into_iter()
            .find(|account| !account.signed_out && Some(account.email.as_str()) != except);
        other.map(|account| Back::SignedIn(account.email))
    }

    /// The sign-in screen, ready to draw.
    fn log_in_screen(&self, ctx: &egui::Context) -> LogIn {
        let mut log_in = LogIn::new(ctx, &self.assets.join("images"), None);
        log_in.wait();
        log_in
    }

    /// Tells the mail on screen the account list, for its switchers, and shows the accounts'
    /// saved profile photos.
    fn share_accounts(&mut self) {
        if self.sign_in.is_none() {
            return;
        }
        let accounts = self.listed();
        if let Some(auth) = self
            .sign_in
            .as_mut()
            .and_then(|sign_in| sign_in.auth().ok())
        {
            self.pictures
                .load(auth, accounts.iter().map(|account| account.email.as_str()));
        }
        if let Screen::Mail(mail) = &mut self.screen {
            mail.set_accounts(accounts);
        }
    }

    /// Carries out a tab key.
    fn tab_command(&mut self, ctx: &egui::Context, command: tabs::Command) {
        match command {
            tabs::Command::New => self.new_tab(ctx),
            tabs::Command::Close => self.close_tab(ctx, self.tabs.active()),
            tabs::Command::Reopen => {
                if let Some(closed) = self.tabs.take_closed() {
                    self.open_tab(ctx, closed.into());
                }
            }
            tabs::Command::Select(index) => self.activate(ctx, index),
            tabs::Command::Next => self.activate(ctx, self.tabs.next()),
            tabs::Command::Previous => self.activate(ctx, self.tabs.previous()),
        }
    }

    /// Shows tab `to`, if it's another: the screen on screen goes with the tab
    /// leaving, as it was, and `to` comes up as it was left.
    fn activate(&mut self, ctx: &egui::Context, to: usize) {
        let Some((from, target)) = self.tabs.select(to) else {
            return;
        };
        let parked = match self.bring(ctx, target) {
            Screen::Mail(page) => Parked::Page(page),
            screen => Parked::Screen(Box::new(screen)),
        };
        self.tabs.park(from, parked);
        ctx.request_repaint();
    }

    /// Puts `parked` up, returning the screen it replaced.
    fn bring(&mut self, ctx: &egui::Context, parked: Parked) -> Screen {
        match parked {
            Parked::Page(page) => self.put(Screen::Mail(page)),
            Parked::Route(route) => self.go(ctx, route),
            Parked::SignIn => {
                let back = self.back(None, None);
                let log_in = self.log_in_screen(ctx).with_back(back);
                self.put(Screen::LogIn(log_in))
            }
            Parked::Screen(screen) => self.put(*screen),
        }
    }

    /// Puts up a page of `route`'s account at the route, a new one unless it's an account
    /// opened at launch and not shown yet, and returns the screen it replaced. An account no
    /// longer signed in on this Mac gives way to the account used last's home, or to the
    /// sign-in screen with none; one that can't be shown gives way to what switching to it
    /// shows.
    fn go(&mut self, ctx: &egui::Context, mut route: Route) -> Screen {
        if let Some(auth) = self
            .sign_in
            .as_mut()
            .and_then(|sign_in| sign_in.auth().ok())
            && !auth
                .accounts()
                .iter()
                .any(|account| account.email == route.email)
        {
            match auth.default_account() {
                Some(account) => route = Route::home(account.email),
                None => return self.bring(ctx, Parked::SignIn),
            }
        }
        let Some(screen) = self.screen_for(ctx, &route.email, false) else {
            return self.bring(ctx, Parked::SignIn);
        };
        let left = self.put(screen);
        if let Screen::Mail(page) = &mut self.screen {
            page.go_to_route(&route);
        }
        left
    }

    /// A new tab (⌘T, `+`): the active tab's account at its home, else the account
    /// used last's, else the sign-in screen; last in the strip, and on screen.
    fn new_tab(&mut self, ctx: &egui::Context) {
        let email = match self.current() {
            Some(email) => Some(email.to_owned()),
            None => self
                .sign_in
                .as_mut()
                .and_then(|sign_in| sign_in.auth().ok())
                .and_then(exo_auth::Auth::default_account)
                .map(|account| account.email),
        };
        self.open_tab(
            ctx,
            email.map_or(Parked::SignIn, |email| Parked::Route(Route::home(email))),
        );
    }

    /// Opens a tab at `parked`, last, and shows it; at the limit the active tab's neighbour closes
    /// first.
    fn open_tab(&mut self, ctx: &egui::Context, parked: Parked) {
        // Until it shows, the active tab's title after its `-`.
        let active = &self.tabs.active_pill().title;
        let title = active.split('-').nth(1).unwrap_or(active).to_owned();
        let pill = chrome::Pill {
            title,
            login_screen: matches!(parked, Parked::SignIn),
            account_color: None,
        };
        let index = self.tabs.push(pill, parked);
        self.activate(ctx, index);
    }

    /// Closes tab `index`: the one on screen gives way to its right neighbour, else its left one;
    /// the last one closes the window, which hides and keeps it.
    /// What closed is what ⌘⇧T opens again.
    fn close_tab(&mut self, ctx: &egui::Context, index: usize) {
        if index != self.tabs.active() {
            if let Some(Parked::Page(page)) = self.tabs.close(index) {
                self.closed(page);
            }
            ctx.request_repaint();
            return;
        }
        let Some(next) = self.tabs.neighbour() else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        };
        let closed = self.mail().map_or(tabs::Closed::SignIn, |mail| {
            tabs::Closed::Route(mail.route())
        });
        let Some((from, target)) = self.tabs.select(next) else {
            return;
        };
        let left = self.bring(ctx, target);
        self.tabs.close_shown(from, closed);
        // The closed tab's own screen goes; its page too, unless its account has no other.
        if let Screen::Mail(page) = left {
            self.closed(page);
        }
        ctx.request_repaint();
    }

    /// Shows the tab for `email`'s mail: the one on screen if it shows that
    /// account, else the first that does, else a new one on it. An account not on this Mac
    /// leaves the tab on screen as it is.
    fn tab_for(&mut self, ctx: &egui::Context, email: &str) {
        if self.current() == Some(email)
            || (self.sign_in.is_some()
                && !self.listed().iter().any(|account| account.email == email))
        {
            return;
        }
        match self.tabs.find(email) {
            Some(index) => self.activate(ctx, index),
            None => self.open_tab(ctx, Parked::Route(Route::home(email.to_owned()))),
        }
    }

    /// Brings the titles and account colours of the tabs not on screen up to date, from their
    /// accounts' latest counts, as their pages would.
    fn refresh_pills(&mut self) {
        for page in self.tabs.pages_mut() {
            page.account_mut().catch_up();
        }
        let listed = self.listed();
        let several = listed.len() > 1;
        let pills: Vec<(usize, Option<String>, String)> = self
            .tabs
            .parked()
            .filter_map(|(index, parked)| {
                let (account, route) = match parked {
                    Parked::Page(page) => (page.account(), page.route()),
                    Parked::Route(route) => (self.account_of(&route.email)?, route.clone()),
                    Parked::SignIn | Parked::Screen(_) => return None,
                };
                let who = if several { route.email.as_str() } else { "Exo" };
                let color = crate::accounts::color_name(
                    &route.email,
                    listed.iter().find(|account| account.email == route.email),
                )
                .to_owned();
                Some((index, tabs::parked_title(account, &route, who), color))
            })
            .collect();
        for (index, title, color) in pills {
            let Some(pill) = self.tabs.pill_mut(index) else {
                continue;
            };
            if let Some(title) = title {
                pill.title = title;
            }
            pill.login_screen = false;
            pill.account_color = Some(color);
        }
    }

    /// Asks Google for the profile photo of the account on screen each time its mail starts: at
    /// launch, on switching and after a sign-in.
    fn refresh_picture(&mut self) {
        let (Screen::Mail(mail), Some(sign_in)) = (&self.screen, &mut self.sign_in) else {
            return;
        };
        let email = mail.account().email();
        let Some((_, tokens)) = self.grants.iter().find(|(open, _)| open == email) else {
            return;
        };
        if let Ok(auth) = sign_in.auth() {
            self.pictures.refresh(auth, email, tokens.clone());
        }
    }
}

impl eframe::App for App {
    /// What goes on whatever state the window is in, hidden or minimized too: the window's
    /// closing and coming back, how often each account syncs, and the Dock badge.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Something kept panicking (`recovery`): the crash dialog, then Exo quits.
        if let Some(gave_up) = recovery::gave_up() {
            match &self.desktop {
                Some(desktop) => desktop.crashed_repeatedly(gave_up),
                None => log::error!(
                    "not quitting after repeated panics: Exo isn't running as a Mac app"
                ),
            }
        }
        let mut mail = match &mut self.screen {
            Screen::Mail(mail) => Some(mail),
            Screen::LogIn(_) | Screen::SignedOut(_) | Screen::Unopened(_) => None,
        };
        let last_tab = self.tabs.len() == 1;
        let hidden = self.desktop.as_mut().is_some_and(|desktop| {
            let hid = desktop.logic(ctx, last_tab);
            desktop.set_badge(
                &mail
                    .as_ref()
                    .and_then(|mail| mail.badge())
                    .unwrap_or_default(),
            );
            // Drafts save a second after their last change, on a frame, and a hidden window has
            // none: what was typed just before closing is saved now.
            if let Some(mail) = mail.as_mut().filter(|_| hid) {
                mail.save_drafts();
            }
            desktop.is_hidden()
        });
        let presence = ctx.input(|input| {
            if hidden || input.viewport().minimized == Some(true) {
                Presence::Background
            } else if input.focused {
                Presence::Focused
            } else {
                Presence::Visible
            }
        });
        if let Some(mail) = mail {
            mail.account().set_presence(presence);
        }
        // The updater's check shows its alerts in the mail on screen, else asks with dialogs.
        if let Some(updates) = &self.updates {
            updates.set_alerts_shown(matches!(self.screen, Screen::Mail(_)) && !hidden);
        }
        // The first frame has drawn: the update that relaunched Exo stays.
        if ctx.cumulative_frame_nr() > 0
            && let Some(receipt) = self.receipt.take()
        {
            std::thread::spawn(move || {
                if let Err(error) = receipt.acknowledge() {
                    log::warn!("finishing the update: {error:#}");
                }
            });
        }
        // An account another tab shows syncs as a hidden window's does, its tab's page connected
        // to its background; the others at the pace of an account no tab shows, whatever the
        // window does.
        for account in self.off_screen() {
            account.set_presence(if self.tabs.shows(account.email()) {
                Presence::Background
            } else {
                Presence::OffScreen
            });
        }
        let on_screen = self.mail().map(Mail::route);
        if let Some(file) = &mut self.tabs_file {
            file.save(&self.tabs, on_screen.as_ref());
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let mut frames = std::mem::take(&mut self.frames);
        frames.draw(&ctx, || self.frame(ui));
        self.frames = frames;
    }

    /// The menu items' keystrokes and macOS's inserted text, for whatever has the keyboard.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(desktop) = &mut self.desktop {
            desktop.input(ctx, raw_input);
        }
    }
}

/// What launch opens.
struct Launch {
    screen: Screen,
    /// The grants of the accounts it opened.
    grants: Vec<(String, Tokens)>,
    background: Vec<Away>,
}

impl Launch {
    /// `screen`, with no account open off it.
    fn screen(screen: Screen) -> Self {
        Self {
            screen,
            grants: Vec::new(),
            background: Vec::new(),
        }
    }
}

/// What sign-in opens with: every
/// account still signed in, the default account's mail on screen and the others open off it.
/// Every grant is read before any store opens, the default's first, so the Keychain's prompts
/// (one an account while Exo is signed ad hoc) come together. On screen: "Almost there…" when
/// the Keychain withholds the default's grant, "Sorry!" when its store can't be opened, or the
/// sign-in screen when there's no account signed in or no OAuth client to sign in with. Another
/// account whose grant or store can't be had stays closed, to show the same when it's switched
/// to.
fn launch(
    sign_in: &mut SignIn,
    ctx: &egui::Context,
    assets: &Path,
    clock: &Clock,
) -> Result<Launch, LaunchError> {
    // The window stays hidden until the first frame paints (eframe), so waiting for the photo
    // here opens it with the photo already there.
    let log_in_screen = |footer| {
        let mut screen = LogIn::new(ctx, &assets.join("images"), footer);
        screen.wait();
        Launch::screen(Screen::LogIn(screen))
    };
    let auth = match sign_in.auth() {
        Ok(auth) => auth.clone(),
        Err(
            error @ (exo_auth::Error::MissingClient { .. } | exo_auth::Error::InvalidClient { .. }),
        ) => {
            log::warn!("{error}");
            return Ok(log_in_screen(log_in::footer(&error)));
        }
        Err(error) => return Err(error.into()),
    };
    let Some(first) = auth.default_account() else {
        return Ok(log_in_screen(None));
    };
    // Every grant before any store, so the Keychain asks for them all at once.
    let first_grant = auth.tokens(&first.email);
    let others: Vec<String> = auth
        .accounts()
        .into_iter()
        .filter(|account| !account.signed_out && account.email != first.email)
        .map(|account| account.email)
        .filter(|email| {
            auth.tokens(email)
                .inspect_err(|error| log::warn!("reading an account's grant: {error}"))
                .is_ok()
        })
        .collect();
    let mut grants = Vec::new();
    let screen = match first_grant
        .map_err(MailError::from)
        .and_then(|_| sign_in.open_mail(&first.email, ctx))
    {
        Ok((account, tokens)) => {
            grants.push((first.email, tokens));
            Screen::Mail(Box::new(Mail::new(ctx, assets, account, clock.clone())))
        }
        // The Keychain won't hand over the grant (say, the user clicked Deny): as with a grant
        // Google refused, the account signs in again, which writes the Keychain item afresh.
        Err(MailError::Auth(error @ exo_auth::Error::Secrets(_))) => {
            log::warn!("reading an account's grant: {error}");
            Screen::SignedOut(SignedOut::new(
                ctx,
                &assets.join("images"),
                &clock.now(),
                first.email,
            ))
        }
        // Its store can't be opened: why, in place of its mail.
        Err(MailError::Open(error)) => {
            log::error!("opening an account: {error}");
            let reason = Reason::of(&error);
            Screen::Unopened(Unopened::new(ctx, &clock.now(), first.email, reason))
        }
        Err(error) => return Err(error.into()),
    };
    let mut background = Vec::new();
    for email in others {
        match sign_in.open_mail(&email, ctx) {
            Ok((account, tokens)) => {
                background.push(Away::opened(account, clock));
                grants.push((email, tokens));
            }
            Err(error) => log::error!("opening an account: {error}"),
        }
    }
    Ok(Launch {
        screen,
        grants,
        background,
    })
}

/// Deletes `email`'s attachment previews and kept parts, caches like its store's mail.
fn remove_previews(email: &str) {
    let previews = crate::attachment_files::default_cache(email);
    for cache in std::iter::once(previews).chain(crate::account::parts_cache(email)) {
        if let Err(error) = std::fs::remove_dir_all(&cache)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::error!("deleting an account's attachment cache: {error}");
        }
    }
}
