pub mod dialogs;
mod fonts;
mod theme;

use crate::Arguments;
use gtk::{gdk, gio, glib, prelude::*};
use safeincloud_linux::{
    drive::{Candidate, Drive, DriveSession},
    favicons, oauth,
    session::{self, Recovery},
    vault::{self, Card, Field, Vault},
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    rc::{Rc, Weak},
    sync::mpsc,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

enum Job {
    Unlock(Result<Vault, String>),
    Sync(Vault, DriveSession, Result<String, String>),
    Discover(Result<Vec<Candidate>, String>),
    Open(Result<DriveSession, String>),
    Icon(String, Option<Vec<u8>>),
}

#[derive(Clone, PartialEq)]
enum Filter {
    All,
    Favorites,
    Label(String),
}

struct State {
    path: Option<PathBuf>,
    vault: Option<Vault>,
    drive: Option<Drive>,
    session: Option<DriveSession>,
    recovery: Option<Recovery>,
    unlocked: bool,
    busy: bool,
    generation: u64,
    selected: Option<String>,
    filter: Filter,
    icons: HashMap<String, Option<gdk::Texture>>,
    pending_icons: HashSet<String>,
    icon_queue: VecDeque<String>,
    active_icons: usize,
    clipboard: Option<gdk::ContentProvider>,
    clipboard_generation: u64,
}

pub struct Ui {
    application: gtk::Application,
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    password: gtk::PasswordEntry,
    unlock_button: gtk::Button,
    unlock_error: gtk::Label,
    source_label: gtk::Label,
    navigation: gtk::ListBox,
    entries: gtk::ListBox,
    detail: gtk::Box,
    search: gtk::SearchEntry,
    status: gtk::Label,
    cloud: gtk::Label,
    count: gtk::Label,
    icons_enabled: gtk::CheckButton,
    sync_button: gtk::Button,
    add_button: gtk::Button,
    generator_button: gtk::Button,
    state: RefCell<State>,
    updating: Cell<bool>,
    activity: Cell<Instant>,
    sender: mpsc::Sender<Job>,
    receiver: RefCell<mpsc::Receiver<Job>>,
    demo_directory: Option<tempfile::TempDir>,
    smoke: bool,
    _appearance: Option<gio::DBusProxy>,
    weak: RefCell<Weak<Ui>>,
}

fn text(value: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(&value.replace('\0', "�")));
    label.set_xalign(0.0);
    if !class.is_empty() {
        label.add_css_class(class);
    }
    label
}

fn action(value: &str, icon: &str) -> gtk::Button {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&gtk::Image::from_icon_name(icon));
    row.append(&text(value, ""));
    gtk::Button::builder().child(&row).build()
}

fn clear(container: &impl IsA<gtk::Widget>) {
    while let Some(child) = container.first_child() {
        if let Some(row) = container.dynamic_cast_ref::<gtk::ListBox>() {
            row.remove(&child);
        } else if let Some(row) = container.dynamic_cast_ref::<gtk::Box>() {
            row.remove(&child);
        } else {
            child.unparent();
        }
    }
}

impl Ui {
    fn new(application: &gtk::Application, mut args: Arguments) -> Rc<Self> {
        let appearance = theme::bind();
        let demo_directory = if args.demo {
            let directory = tempfile::tempdir().expect("Demo-Verzeichnis");
            let path = directory.path().join("demo.db");
            std::fs::write(
                &path,
                vault::encrypt(
                    include_bytes!("../../assets/demo.xml"),
                    "synthetic-demo-only",
                    2,
                )
                .expect("Demo verschlüsseln"),
            )
            .expect("Demo speichern");
            args.path = Some(path);
            Some(directory)
        } else {
            None
        };
        let drive = if args.demo { None } else { Drive::new().ok() };
        let cached = if args.path.is_none() {
            drive
                .as_ref()
                .and_then(|d| d.cached_session().ok().flatten())
        } else {
            drive
                .as_ref()
                .and_then(|d| d.cached_session().ok().flatten())
                .filter(|s| Some(&s.path) == args.path.as_ref())
        };
        if args.path.is_none() {
            args.path = cached.as_ref().map(|s| s.path.clone());
        }
        let restored_session = args
            .recovery
            .as_ref()
            .and_then(|r| r.sync.as_ref())
            .and_then(|s| {
                drive
                    .as_ref()?
                    .restore_session(args.path.clone()?, s.clone())
                    .ok()
            });
        let window = gtk::ApplicationWindow::builder()
            .application(application)
            .title("SafeInCloud Linux")
            .default_width(1320)
            .default_height(820)
            .build();
        window.set_size_request(940, 640);
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(140)
            .build();
        window.set_child(Some(&stack));
        let unlock = gtk::Box::new(gtk::Orientation::Vertical, 18);
        unlock.set_valign(gtk::Align::Center);
        unlock.set_halign(gtk::Align::Center);
        unlock.set_width_request(420);
        unlock.add_css_class("unlock-card");
        let mark = gtk::Image::from_icon_name("dialog-password-symbolic");
        mark.set_pixel_size(36);
        mark.set_halign(gtk::Align::Start);
        unlock.append(&mark);
        unlock.append(&text("SafeInCloud Linux", "heading"));
        unlock.append(&text(
            "Deine Passwörter. Auf deinem Linux-Desktop.",
            "muted",
        ));
        let source_label = text(
            "Lokale Datenbank öffnen oder Google Drive verbinden.",
            "muted",
        );
        source_label.set_wrap(true);
        unlock.append(&source_label);
        let password = gtk::PasswordEntry::builder()
            .placeholder_text("Masterpasswort")
            .show_peek_icon(true)
            .activates_default(true)
            .build();
        password.set_widget_name("master-password");
        unlock.append(&password);
        let unlock_button = gtk::Button::with_label("Datenbank entsperren");
        unlock_button.add_css_class("suggested-action");
        unlock_button.set_widget_name("unlock");
        unlock.append(&unlock_button);
        let sources = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let local = gtk::Button::with_label("Datei öffnen");
        local.set_hexpand(true);
        let connect = gtk::Button::with_label("Google Drive verbinden");
        connect.set_hexpand(true);
        sources.append(&local);
        sources.append(&connect);
        unlock.append(&sources);
        let unlock_error = text(args.notice.as_deref().unwrap_or(""), "muted");
        unlock_error.set_wrap(true);
        unlock_error.set_max_width_chars(48);
        unlock.append(&unlock_error);
        stack.add_named(&unlock, Some("locked"));

        let main = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar.set_width_request(220);
        sidebar.add_css_class("sidebar");
        let brand = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        brand.add_css_class("pane-header");
        brand.append(&gtk::Image::from_icon_name("dialog-password-symbolic"));
        brand.append(&text("SafeInCloud Linux", "brand"));
        sidebar.append(&brand);
        let navigation = gtk::ListBox::new();
        navigation.add_css_class("navigation");
        let nav_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&navigation)
            .build();
        sidebar.append(&nav_scroll);
        let icons_enabled = gtk::CheckButton::with_label("Website-Icons");
        icons_enabled.set_active(!args.smoke && load_icons_preference());
        icons_enabled.set_tooltip_text(Some("Icons direkt per HTTPS von der Website und ihren Bildhosts laden. Der Cache bleibt bis zum Sperren im Arbeitsspeicher."));
        icons_enabled.set_margin_start(14);
        icons_enabled.set_margin_bottom(12);
        sidebar.append(&icons_enabled);
        let cloud = text("", "muted");
        cloud.set_wrap(true);
        cloud.set_margin_start(16);
        cloud.set_margin_end(14);
        cloud.set_margin_bottom(16);
        sidebar.append(&cloud);
        main.append(&sidebar);
        let split = gtk::Paned::new(gtk::Orientation::Horizontal);
        split.set_hexpand(true);
        split.set_wide_handle(false);
        split.set_resize_start_child(false);
        split.set_shrink_start_child(false);
        split.set_shrink_end_child(false);
        let middle = gtk::Box::new(gtk::Orientation::Vertical, 0);
        middle.set_width_request(290);
        middle.add_css_class("list-pane");
        let search_header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        search_header.add_css_class("pane-header");
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Suchen"));
        search.set_hexpand(true);
        search.set_widget_name("search");
        let add = gtk::Button::from_icon_name("list-add-symbolic");
        add.add_css_class("flat");
        add.set_tooltip_text(Some("Neues Passwort (Strg+N)"));
        search_header.append(&search);
        search_header.append(&add);
        middle.append(&search_header);
        let entries = gtk::ListBox::new();
        entries.add_css_class("entries");
        entries.set_activate_on_single_click(false);
        middle.append(
            &gtk::ScrolledWindow::builder()
                .vexpand(true)
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&entries)
                .build(),
        );
        let count = text("", "muted");
        count.set_margin_start(16);
        count.set_margin_top(8);
        count.set_margin_bottom(10);
        middle.append(&count);
        split.set_start_child(Some(&middle));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_hexpand(true);
        content.add_css_class("content");
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toolbar.add_css_class("pane-header");
        toolbar.set_halign(gtk::Align::Fill);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        toolbar.append(&spacer);
        let generator = action("Generator", "dialog-password-symbolic");
        generator.add_css_class("flat");
        toolbar.append(&generator);
        let sync_button = action("Drive abgleichen", "view-refresh-symbolic");
        sync_button.add_css_class("flat");
        toolbar.append(&sync_button);
        let lock = action("Sperren", "system-lock-screen-symbolic");
        lock.add_css_class("flat");
        toolbar.append(&lock);
        content.append(&toolbar);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 20);
        detail.set_margin_start(32);
        detail.set_margin_end(32);
        detail.set_margin_top(30);
        detail.set_margin_bottom(24);
        detail.set_halign(gtk::Align::Fill);
        detail.set_hexpand(true);
        detail.set_width_request(340);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&detail)
            .build();
        content.append(&scroll);
        let status = text("", "muted");
        status.set_wrap(true);
        status.set_margin_start(22);
        status.set_margin_end(22);
        status.set_margin_bottom(14);
        content.append(&status);
        split.set_end_child(Some(&content));
        split.set_position(330);
        main.append(&split);
        stack.add_named(&main, Some("main"));
        stack.set_visible_child_name("locked");
        let (sender, receiver) = mpsc::channel();
        let ui = Rc::new(Self {
            application: application.clone(),
            window,
            stack,
            password,
            unlock_button,
            unlock_error,
            source_label,
            navigation,
            entries,
            detail,
            search,
            status,
            cloud,
            count,
            icons_enabled,
            sync_button,
            add_button: add.clone(),
            generator_button: generator.clone(),
            state: RefCell::new(State {
                path: args.path,
                vault: None,
                drive,
                session: restored_session.or(cached),
                recovery: args.recovery,
                unlocked: false,
                busy: false,
                generation: 0,
                selected: if args.demo {
                    Some("github".into())
                } else {
                    None
                },
                filter: Filter::All,
                icons: HashMap::new(),
                pending_icons: HashSet::new(),
                icon_queue: VecDeque::new(),
                active_icons: 0,
                clipboard: None,
                clipboard_generation: 0,
            }),
            updating: Cell::new(false),
            activity: Cell::new(Instant::now()),
            sender,
            receiver: RefCell::new(receiver),
            demo_directory,
            smoke: args.smoke,
            _appearance: appearance,
            weak: RefCell::new(Weak::new()),
        });
        ui.weak.replace(Rc::downgrade(&ui));
        macro_rules! click { ($button:expr, $method:ident $(,$arg:expr)*) => {{ let weak=Rc::downgrade(&ui); $button.connect_clicked(move |_| { if let Some(ui)=weak.upgrade() { ui.$method($($arg),*); } }); }}; }
        click!(ui.unlock_button, unlock);
        click!(local, choose_local);
        click!(connect, connect_drive);
        click!(add, edit, true);
        click!(generator, generate);
        click!(lock, lock);
        click!(ui.sync_button, sync);
        {
            let weak = Rc::downgrade(&ui);
            ui.password.connect_activate(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.unlock();
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.search.connect_search_changed(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.refresh_entries();
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.entries.connect_row_selected(move |_, row| {
                if let Some(ui) = weak.upgrade() {
                    if !ui.updating.get() {
                        ui.state.borrow_mut().selected = row.map(|r| r.widget_name().to_string());
                        ui.show_detail();
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.entries.connect_row_activated(move |_, _| {
                if let Some(ui) = weak.upgrade() {
                    ui.edit(false);
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.navigation.connect_row_selected(move |_, row| {
                if let Some(ui) = weak.upgrade() {
                    if !ui.updating.get() {
                        if let Some(row) = row {
                            let name = row.widget_name();
                            ui.state.borrow_mut().filter = match name.as_str() {
                                "all" => Filter::All,
                                "favorites" => Filter::Favorites,
                                _ => Filter::Label(name.to_string()),
                            };
                            ui.refresh_entries();
                        }
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.icons_enabled.connect_toggled(move |control| {
                if let Some(ui) = weak.upgrade() {
                    if !ui.smoke {
                        save_icons_preference(control.is_active());
                    }
                    {
                        let mut s = ui.state.borrow_mut();
                        s.icons.clear();
                        while let Some(origin) = s.icon_queue.pop_front() {
                            s.pending_icons.remove(&origin);
                        }
                    }
                    ui.refresh_entries();
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            glib::timeout_add_local(Duration::from_millis(40), move || {
                if let Some(ui) = weak.upgrade() {
                    ui.poll();
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            glib::timeout_add_local(Duration::from_secs(5), move || {
                if let Some(ui) = weak.upgrade() {
                    if ui.state.borrow().unlocked
                        && ui.activity.get().elapsed() >= Duration::from_secs(300)
                    {
                        ui.lock();
                    }
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.window.connect_close_request(move |_| {
                let Some(ui) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let pending = {
                    let s = ui.state.borrow();
                    s.recovery.is_some() || s.vault.as_ref().is_some_and(|v| v.dirty)
                };
                if pending {
                    ui.confirm_close();
                    glib::Propagation::Stop
                } else {
                    ui.clear_clipboard();
                    glib::Propagation::Proceed
                }
            });
        }
        ui.install_activity(ui.window.upcast_ref());
        {
            let weak = Rc::downgrade(&ui);
            application.connect_window_added(move |_, window| {
                if let Some(ui) = weak.upgrade() {
                    ui.install_activity(window);
                }
            });
        }
        ui.shortcuts();
        ui.update_source();
        if ui.demo_directory.is_some() {
            ui.password.set_text("synthetic-demo-only");
            ui.unlock();
        }
        ui.window.present();
        if let Some(destination) = args.screenshot {
            let weak = Rc::downgrade(&ui);
            glib::timeout_add_local_once(Duration::from_secs(3), move || {
                if let Some(ui) = weak.upgrade() {
                    ui.screenshot(&destination);
                    ui.application.quit();
                }
            });
        }
        if ui.smoke {
            let weak = Rc::downgrade(&ui);
            let started = Instant::now();
            glib::timeout_add_local(Duration::from_millis(100), move || {
                let Some(ui) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if ui.state.borrow().unlocked {
                    ui.smoke_test();
                    ui.application.quit();
                    return glib::ControlFlow::Break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(20),
                    "Demo unlock timed out"
                );
                glib::ControlFlow::Continue
            });
        }
        ui
    }

    fn install_activity(&self, window: &gtk::Window) {
        let controller = gtk::EventControllerLegacy::new();
        let weak = self.weak.borrow().clone();
        controller.connect_event(move |_, event| {
            if matches!(
                event.event_type(),
                gdk::EventType::ButtonPress
                    | gdk::EventType::KeyPress
                    | gdk::EventType::Scroll
                    | gdk::EventType::TouchBegin
            ) {
                if let Some(ui) = weak.upgrade() {
                    ui.activity.set(Instant::now());
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(controller);
    }

    fn shortcuts(&self) {
        for (name, keys) in [
            ("search", &["<Primary>f"][..]),
            ("new", &["<Primary>n"][..]),
            ("edit", &["<Primary>e"][..]),
            ("generator", &["<Primary>g"][..]),
            ("lock", &["<Primary>l"][..]),
        ] {
            let action = gio::SimpleAction::new(name, None);
            let weak = self.weak.borrow().clone();
            let name = name.to_owned();
            action.connect_activate(move |_, _| {
                if let Some(ui) = weak.upgrade() {
                    match name.as_str() {
                        "search" => {
                            ui.search.grab_focus();
                        }
                        "new" => ui.edit(true),
                        "edit" => ui.edit(false),
                        "generator" => ui.generate(),
                        "lock" => ui.lock(),
                        _ => {}
                    }
                }
            });
            self.application.add_action(&action);
            self.application
                .set_accels_for_action(&format!("app.{}", action.name()), keys);
        }
    }

    fn update_source(&self) {
        let s = self.state.borrow();
        let editable = s.unlocked && !s.busy;
        self.add_button.set_sensitive(editable);
        self.generator_button.set_sensitive(editable);
        self.navigation.set_sensitive(editable);
        self.search.set_sensitive(editable);
        self.entries.set_sensitive(editable);
        set_editable(self.detail.upcast_ref(), editable);
        for name in ["search", "new", "edit", "generator"] {
            if let Some(action) = self
                .application
                .lookup_action(name)
                .and_downcast::<gio::SimpleAction>()
            {
                action.set_enabled(editable);
            }
        }
        self.source_label.set_text(
            &s.path
                .as_ref()
                .map(|p| {
                    p.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                })
                .unwrap_or_else(|| "Lokale Datenbank öffnen oder Google Drive verbinden.".into()),
        );
        self.unlock_button
            .set_sensitive(s.path.is_some() && !s.busy);
        self.password.set_sensitive(!s.busy);
        self.sync_button.set_visible(s.session.is_some());
        self.sync_button.set_sensitive(s.unlocked && !s.busy);
        self.cloud.set_text(if s.session.is_some() {
            "● Google Drive\nVerschlüsselte lokale Kopie"
        } else {
            "● Lokale Datenbank\nAuf diesem Gerät gespeichert"
        });
    }

    fn unlock(&self) {
        let (path, recovery) = {
            let mut s = self.state.borrow_mut();
            if s.busy || s.unlocked {
                return;
            }
            let Some(path) = s.path.clone() else {
                return;
            };
            s.busy = true;
            (path, s.recovery.clone())
        };
        let password = Zeroizing::new(self.password.text().to_string());
        self.password.set_text("");
        self.unlock_error.set_text("Datenbank wird entsperrt …");
        self.update_source();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = if let Some(r) = recovery {
                r.unlock(&path, &password)
            } else {
                Vault::open(&path, &password)
            }
            .map_err(|e| e.to_string());
            let _ = sender.send(Job::Unlock(result));
        });
    }

    fn poll(&self) {
        loop {
            let job = self.receiver.borrow_mut().try_recv();
            let Ok(job) = job else {
                break;
            };
            match job {
                Job::Unlock(result) => {
                    self.state.borrow_mut().busy = false;
                    match result {
                        Ok(vault) => {
                            {
                                let mut s = self.state.borrow_mut();
                                s.vault = Some(vault);
                                s.unlocked = true;
                                s.recovery = None;
                            }
                            self.activity.set(Instant::now());
                            self.stack.set_visible_child_name("main");
                            self.status.set_text(
                                "Entsperrt · Änderungen werden verschlüsselt lokal gespeichert",
                            );
                            self.refresh();
                            if !self.smoke {
                                self.sync();
                            }
                        }
                        Err(error) => self.unlock_error.set_text(&error),
                    }
                    self.update_source();
                }
                Job::Sync(vault, session, result) => {
                    {
                        let mut s = self.state.borrow_mut();
                        s.vault = Some(vault);
                        s.session = Some(session);
                        s.busy = false;
                        s.recovery = None;
                    }
                    self.status.set_text(&result.unwrap_or_else(|e| e));
                    self.refresh();
                    self.update_source();
                }
                Job::Discover(result) => {
                    self.state.borrow_mut().busy = false;
                    match result {
                        Ok(candidates) => self.select_candidate(candidates),
                        Err(error) => self.unlock_error.set_text(&error),
                    }
                    self.update_source();
                }
                Job::Open(result) => {
                    self.state.borrow_mut().busy = false;
                    match result {
                        Ok(session) => {
                            let mut s = self.state.borrow_mut();
                            s.path = Some(session.path.clone());
                            s.session = Some(session);
                            self.unlock_error
                                .set_text("Drive-Datei bereit. Bitte Masterpasswort eingeben.");
                        }
                        Err(error) => self.unlock_error.set_text(&error),
                    }
                    self.update_source();
                }
                Job::Icon(origin, bytes) => {
                    let texture = bytes
                        .and_then(|b| gdk::Texture::from_bytes(&glib::Bytes::from_owned(b)).ok());
                    {
                        let mut s = self.state.borrow_mut();
                        s.active_icons = s.active_icons.saturating_sub(1);
                        s.pending_icons.remove(&origin);
                        if self.icons_enabled.is_active() && s.unlocked {
                            s.icons.insert(origin.clone(), texture);
                        }
                    }
                    self.update_icon_widgets(&origin);
                    self.pump_icons();
                }
            }
        }
    }

    fn refresh(&self) {
        self.refresh_navigation();
        self.refresh_entries();
    }

    fn refresh_navigation(&self) {
        let (cards, labels, filter) = {
            let s = self.state.borrow();
            let Some(v) = s.vault.as_ref() else {
                return;
            };
            (v.cards(), v.labels(), s.filter.clone())
        };
        self.updating.set(true);
        clear(&self.navigation);
        let mut rows = vec![
            (
                "all".to_owned(),
                "Alle Einträge".to_owned(),
                cards.len(),
                "view-list-symbolic",
            ),
            (
                "favorites".to_owned(),
                "Favoriten".to_owned(),
                cards.iter().filter(|c| c.favorite).count(),
                "starred-symbolic",
            ),
        ];
        for label in labels {
            let count = cards
                .iter()
                .filter(|c| c.label_ids.contains(&label.id))
                .count();
            rows.push((label.id, label.name, count, "folder-symbolic"));
        }
        for (id, name, count, icon) in rows {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            line.set_margin_start(12);
            line.set_margin_end(12);
            line.set_margin_top(10);
            line.set_margin_bottom(10);
            line.append(&gtk::Image::from_icon_name(icon));
            let title = text(&name, "");
            title.set_hexpand(true);
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&title);
            line.append(&text(&count.to_string(), "muted"));
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&id);
            row.set_child(Some(&line));
            self.navigation.append(&row);
            if (filter == Filter::All && id == "all")
                || (filter == Filter::Favorites && id == "favorites")
                || matches!(&filter,Filter::Label(value)if value==&id)
            {
                self.navigation.select_row(Some(&row));
            }
        }
        self.updating.set(false);
    }

    fn refresh_entries(&self) {
        let (mut cards, selected, filter) = {
            let s = self.state.borrow();
            if !s.unlocked {
                return;
            }
            let Some(v) = s.vault.as_ref() else {
                return;
            };
            (v.cards(), s.selected.clone(), s.filter.clone())
        };
        let query = self.search.text().to_lowercase();
        cards.retain(|c| {
            let filter = match &filter {
                Filter::All => true,
                Filter::Favorites => c.favorite,
                Filter::Label(id) => c.label_ids.contains(id),
            };
            filter
                && (query.is_empty()
                    || c.title.to_lowercase().contains(&query)
                    || c.fields
                        .iter()
                        .filter(|f| {
                            matches!(
                                f.kind.to_lowercase().as_str(),
                                "login" | "email" | "website" | "url"
                            )
                        })
                        .any(|f| f.value.to_lowercase().contains(&query)))
        });
        cards.sort_by_key(|c| c.title.to_lowercase());
        self.updating.set(true);
        clear(&self.entries);
        let mut selection = None;
        for card in &cards {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            line.set_margin_start(12);
            line.set_margin_end(12);
            line.set_margin_top(11);
            line.set_margin_bottom(11);
            line.append(&self.avatar(card, 36));
            let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
            labels.set_hexpand(true);
            let title = text(&card.title, "entry-title");
            title.set_ellipsize(gtk::pango::EllipsizeMode::End);
            labels.append(&title);
            let login = card
                .fields
                .iter()
                .find(|f| matches!(f.kind.to_lowercase().as_str(), "login" | "email"))
                .map(|f| f.value.as_str())
                .unwrap_or("Passwort-Eintrag");
            let login = text(login, "muted");
            login.set_ellipsize(gtk::pango::EllipsizeMode::End);
            labels.append(&login);
            line.append(&labels);
            if card.favorite {
                line.append(&gtk::Image::from_icon_name("starred-symbolic"));
            }
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(&card.id);
            row.set_child(Some(&line));
            self.entries.append(&row);
            if Some(&card.id) == selected.as_ref() {
                selection = Some(row);
            }
        }
        let selection = selection.or_else(|| self.entries.row_at_index(0));
        self.entries.select_row(selection.as_ref());
        self.state.borrow_mut().selected = selection.map(|r| r.widget_name().to_string());
        self.updating.set(false);
        self.count.set_text(&format!(
            "{} {}",
            cards.len(),
            if cards.len() == 1 {
                "Eintrag"
            } else {
                "Einträge"
            }
        ));
        self.show_detail();
        self.pump_icons();
    }

    fn current_card(&self) -> Option<Card> {
        let s = self.state.borrow();
        s.vault
            .as_ref()?
            .cards()
            .into_iter()
            .find(|c| Some(&c.id) == s.selected.as_ref())
    }

    fn avatar(&self, card: &Card, size: i32) -> gtk::Box {
        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        box_.set_size_request(size, size);
        box_.set_hexpand(false);
        box_.set_valign(gtk::Align::Center);
        box_.add_css_class("avatar");
        let origin = if self.icons_enabled.is_active() {
            card.fields
                .iter()
                .filter(|f| matches!(f.kind.to_lowercase().as_str(), "website" | "url"))
                .find_map(|f| favicons::website_origin(&f.value))
        } else {
            None
        };
        let texture = origin
            .as_ref()
            .and_then(|o| self.state.borrow().icons.get(o).cloned().flatten());
        if let Some(texture) = texture {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_size_request(size - 8, size - 8);
            picture.set_margin_start(4);
            picture.set_margin_end(4);
            picture.set_margin_top(4);
            picture.set_margin_bottom(4);
            picture.set_can_shrink(true);
            box_.append(&picture);
            box_.add_css_class("website-icon");
        } else {
            let letter = text(
                &card
                    .title
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string(),
                "",
            );
            letter.set_xalign(0.5);
            letter.set_hexpand(true);
            box_.append(&letter);
        }
        if let Some(origin) = origin {
            box_.set_widget_name(&format!("icon:{origin}"));
            let mut s = self.state.borrow_mut();
            if !s.icons.contains_key(&origin) && s.pending_icons.insert(origin.clone()) {
                s.icon_queue.push_back(origin);
            }
        }
        box_
    }

    fn show_detail(&self) {
        clear(&self.detail);
        let Some(card) = self.current_card() else {
            self.detail
                .append(&text("Deine Passwörter, übersichtlich.", "heading"));
            self.detail.append(&text(
                "Wähle einen Eintrag aus oder füge dein erstes Passwort hinzu.",
                "muted",
            ));
            return;
        };
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        header.append(&self.avatar(&card, 60));
        let title = text(&card.title, "detail-title");
        title.set_wrap(true);
        title.set_hexpand(true);
        header.append(&title);
        let edit = action("Bearbeiten", "document-edit-symbolic");
        edit.add_css_class("requires-edit");
        edit.set_sensitive(!self.state.borrow().busy);
        let weak = self.weak.borrow().clone();
        edit.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.edit(false);
            }
        });
        header.append(&edit);
        self.detail.append(&header);
        self.detail.append(&text("Zugangsdaten", "section-title"));
        let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
        group.add_css_class("card");
        for field in &card.fields {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("field");
            let values = gtk::Box::new(gtk::Orientation::Vertical, 4);
            values.set_hexpand(true);
            let public = matches!(
                field.kind.to_lowercase().as_str(),
                "login" | "email" | "website" | "url"
            );
            values.append(&text(&field.name, "muted"));
            let value = gtk::Button::with_label(if public {
                &field.value
            } else {
                "••••••••••••"
            });
            value.set_widget_name(if public {
                "public-value"
            } else {
                "secret-value"
            });
            value.add_css_class("credential-value");
            value.add_css_class("flat");
            if !public {
                value.add_css_class("secret");
            }
            value.set_tooltip_text(Some(
                "Zum Kopieren klicken · Zwischenablage wird nach 30 Sekunden geleert",
            ));
            value.set_halign(gtk::Align::Fill);
            if let Some(label) = value.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                label.set_max_width_chars(60);
            }
            let weak = self.weak.borrow().clone();
            let secret = Zeroizing::new(field.value.clone());
            value.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.copy_value(&secret);
                }
            });
            values.append(&value);
            if field.kind.eq_ignore_ascii_case("password") {
                let login = card
                    .fields
                    .iter()
                    .find(|f| matches!(f.kind.to_lowercase().as_str(), "login" | "email"))
                    .map(|f| f.value.as_str())
                    .unwrap_or("");
                values.append(&dialogs::strength_widget(
                    &field.value,
                    &[&card.title, login],
                ));
            }
            row.append(&values);
            if !public {
                let reveal = gtk::ToggleButton::with_label("Anzeigen");
                reveal.add_css_class("flat");
                let label = value.clone();
                let weak = self.weak.borrow().clone();
                let secret = Zeroizing::new(field.value.clone());
                reveal.connect_toggled(move |toggle| {
                    if let Some(ui) = weak.upgrade() {
                        if ui.state.borrow().unlocked {
                            label.set_label(if toggle.is_active() {
                                &secret
                            } else {
                                "••••••••••••"
                            });
                            toggle.set_label(if toggle.is_active() {
                                "Verbergen"
                            } else {
                                "Anzeigen"
                            });
                        }
                    }
                });
                row.append(&reveal);
            }
            let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
            copy.add_css_class("flat");
            copy.set_tooltip_text(Some("Kopieren"));
            let weak = self.weak.borrow().clone();
            let secret = Zeroizing::new(field.value.clone());
            copy.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.copy_value(&secret);
                }
            });
            row.append(&copy);
            group.append(&row);
        }
        self.detail.append(&group);
        let metadata = gtk::Box::new(gtk::Orientation::Vertical, 14);
        metadata.add_css_class("card");
        metadata.add_css_class("metadata");
        let label_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let name = text("Label", "");
        name.set_hexpand(true);
        label_row.append(&name);
        let names = {
            let s = self.state.borrow();
            s.vault
                .as_ref()
                .map(|v| {
                    v.labels()
                        .into_iter()
                        .filter(|l| card.label_ids.contains(&l.id))
                        .map(|l| l.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default()
        };
        label_row.append(&text(
            if names.is_empty() {
                "Ohne Label"
            } else {
                &names
            },
            "muted",
        ));
        metadata.append(&label_row);
        let favorite_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let label = text("Favorit", "");
        label.set_hexpand(true);
        favorite_row.append(&label);
        let favorite = gtk::Switch::new();
        favorite.add_css_class("requires-edit");
        favorite.set_active(card.favorite);
        favorite.set_sensitive(!self.state.borrow().busy);
        let weak = self.weak.borrow().clone();
        let id = card.id.clone();
        favorite.connect_state_set(move |_, checked| {
            if let Some(ui) = weak.upgrade() {
                let result = {
                    let mut s = ui.state.borrow_mut();
                    s.vault.as_mut().map(|v| v.set_favorite(&id, checked))
                };
                if let Some(result) = result {
                    match result {
                        Ok(_) => ui.persist(),
                        Err(e) => ui.status.set_text(&e.to_string()),
                    }
                }
            }
            glib::Propagation::Proceed
        });
        favorite_row.append(&favorite);
        metadata.append(&favorite_row);
        self.detail.append(&metadata);
        if !card.notes.is_empty() {
            self.detail.append(&text("Notiz", "section-title"));
            let notes = text(&card.notes, "note");
            notes.set_wrap(true);
            notes.set_selectable(true);
            notes.add_css_class("card");
            self.detail.append(&notes);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_vexpand(true);
        self.detail.append(&spacer);
        let delete = gtk::Button::with_label("Eintrag löschen");
        delete.add_css_class("requires-edit");
        delete.add_css_class("flat");
        delete.add_css_class("destructive-action");
        delete.set_halign(gtk::Align::End);
        delete.set_sensitive(!self.state.borrow().busy);
        let weak = self.weak.borrow().clone();
        delete.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.delete();
            }
        });
        self.detail.append(&delete);
    }

    fn guard(&self) -> dialogs::Guard {
        let weak = self.weak.borrow().clone();
        let generation = self.state.borrow().generation;
        Rc::new(move || {
            weak.upgrade().is_some_and(|ui| {
                ui.activity.set(Instant::now());
                let s = ui.state.borrow();
                s.unlocked && !s.busy && s.generation == generation
            })
        })
    }
    fn copy_callback(&self) -> dialogs::CopyCallback {
        let weak = self.weak.borrow().clone();
        Rc::new(move |value| {
            if let Some(ui) = weak.upgrade() {
                ui.copy_value(value);
            }
        })
    }
    fn generate(&self) {
        if !(self.guard())() {
            return;
        }
        let dialog = dialogs::generator(
            self.window.upcast_ref(),
            self.guard(),
            self.copy_callback(),
            None,
        );
        dialog.present();
    }
    fn edit(&self, new: bool) {
        if !(self.guard())() {
            return;
        }
        let card = if new { None } else { self.current_card() };
        if !new && card.is_none() {
            return;
        }
        let id = card.as_ref().map(|c| c.id.clone());
        let weak = self.weak.borrow().clone();
        let generation = self.state.borrow().generation;
        let save = Rc::new(move |title: String, fields: Vec<Field>, notes: String| {
            let Some(ui) = weak.upgrade() else {
                return false;
            };
            let result = {
                let mut s = ui.state.borrow_mut();
                if !s.unlocked || s.busy || s.generation != generation {
                    return false;
                }
                s.vault
                    .as_mut()
                    .map(|v| v.upsert(id.as_deref(), &title, &fields, &notes))
            };
            if let Some(result) = result {
                match result {
                    Ok(id) => {
                        ui.state.borrow_mut().selected = Some(id);
                        ui.persist();
                        true
                    }
                    Err(e) => {
                        ui.status.set_text(&e.to_string());
                        false
                    }
                }
            } else {
                false
            }
        });
        let dialog = dialogs::editor(
            self.window.upcast_ref(),
            card,
            self.guard(),
            save,
            self.copy_callback(),
        );
        dialog.present();
    }

    fn persist(&self) {
        let result = {
            let mut s = self.state.borrow_mut();
            s.vault.as_mut().map(Vault::save)
        };
        match result {
            Some(Ok(_)) => {
                self.status
                    .set_text("Lokal gespeichert · verschlüsselte Sicherung erstellt");
                self.state.borrow_mut().recovery = None;
                self.refresh();
                self.sync();
            }
            Some(Err(e)) => {
                self.status.set_text(&format!("Speichern fehlgeschlagen. Änderungen bleiben bis zum Speichern verschlüsselt wiederherstellbar: {e}"));
                self.refresh();
            }
            None => {}
        }
    }
    fn sync(&self) {
        let pair = {
            let mut s = self.state.borrow_mut();
            if !s.unlocked || s.busy || s.session.is_none() {
                return;
            }
            let Some(mut vault) = s.vault.take() else {
                return;
            };
            let session = s.session.take().unwrap();
            if vault.dirty {
                // Checkpoint before dispatch: lock may exec the process while
                // the worker is waiting on the network. Its disk baseline must
                // never advance behind an older in-memory recovery snapshot.
                if let Err(error) = vault.save() {
                    s.vault = Some(vault);
                    s.session = Some(session);
                    self.status
                            .set_text(&format!("Drive-Abgleich ausgesetzt: Änderungen konnten nicht lokal gespeichert werden. {error}"));
                    return;
                }
            }
            s.recovery = None;
            debug_assert!(!vault.dirty);
            s.busy = true;
            (vault, session)
        };
        self.status.set_text("Drive wird abgeglichen …");
        self.update_source();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let (mut vault, mut session) = pair;
            let result = session.sync(&mut vault).map_err(|e| e.to_string());
            let _ = sender.send(Job::Sync(vault, session, result));
        });
    }

    fn copy_value(&self, value: &str) {
        if !self.state.borrow().unlocked {
            return;
        }
        self.activity.set(Instant::now());
        let provider = gdk::ContentProvider::for_value(&value.to_value());
        let clipboard = self.window.clipboard();
        if clipboard.set_content(Some(&provider)).is_err() {
            return;
        }
        let generation = {
            let mut s = self.state.borrow_mut();
            s.clipboard = Some(provider);
            s.clipboard_generation += 1;
            s.clipboard_generation
        };
        self.status
            .set_text("Kopiert · Zwischenablage wird nach 30 Sekunden geleert");
        let weak = self.weak.borrow().clone();
        glib::timeout_add_local_once(Duration::from_secs(30), move || {
            if let Some(ui) = weak.upgrade() {
                if ui.state.borrow().clipboard_generation == generation {
                    ui.clear_clipboard();
                }
            }
        });
    }
    fn clear_clipboard(&self) {
        let provider = self.state.borrow_mut().clipboard.take();
        let clipboard = self.window.clipboard();
        if provider.is_some() && clipboard.content() == provider {
            let _ = clipboard.set_content(None::<&gdk::ContentProvider>);
        }
    }

    fn lock(&self) {
        let (path, recovery) = {
            let s = self.state.borrow();
            if !s.unlocked {
                return;
            }
            let recovery = if let Some(v) = s.vault.as_ref().filter(|v| v.dirty) {
                match Recovery::capture(v, s.session.as_ref().map(DriveSession::state)) {
                    Ok(r) => Some(r),
                    Err(_) => {
                        drop(s);
                        self.stack.set_visible_child_name("locked");
                        self.clear_clipboard();
                        session::restart_locked(self.state.borrow().path.as_deref(),None,Some("Die Sitzung wurde gesperrt. Nicht gespeicherte Änderungen konnten nicht wiederhergestellt werden."));
                    }
                }
            } else {
                s.recovery.clone()
            };
            (s.path.clone(), recovery)
        };
        self.stack.set_visible_child_name("locked");
        self.password.set_text("");
        self.clear_clipboard();
        let primary = gtk::prelude::WidgetExt::display(&self.window).primary_clipboard();
        if primary.is_local() {
            let _ = primary.set_content(None::<&gdk::ContentProvider>);
        }
        if self.smoke {
            let mut s = self.state.borrow_mut();
            s.unlocked = false;
            s.generation += 1;
            s.vault = None;
            s.recovery = recovery;
            s.icons.clear();
            drop(s);
            for window in self.application.windows() {
                if window != self.window.clone().upcast::<gtk::Window>() {
                    window.destroy();
                }
            }
            clear(&self.entries);
            clear(&self.navigation);
            clear(&self.detail);
            return;
        }
        let notice=recovery.as_ref().map(|_|"Nicht gespeicherte Änderungen sind verschlüsselt erhalten. Zum Wiederherstellen erneut entsperren und die App bis zum Speichern geöffnet lassen.");
        session::restart_locked(path.as_deref(), recovery.as_ref(), notice);
    }

    fn delete(&self) {
        let Some(card) = self.current_card() else {
            return;
        };
        let guard = self.guard();
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .text("Eintrag löschen?")
            .secondary_text("Der Eintrag wird in den Papierkorb verschoben.")
            .build();
        dialog.add_button("Abbrechen", gtk::ResponseType::Cancel);
        dialog.add_button("Löschen", gtk::ResponseType::Accept);
        let weak = self.weak.borrow().clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept && guard() {
                if let Some(ui) = weak.upgrade() {
                    let result = ui
                        .state
                        .borrow_mut()
                        .vault
                        .as_mut()
                        .map(|v| v.delete(&card.id));
                    if let Some(Ok(_)) = result {
                        ui.persist();
                    }
                }
            }
            dialog.close();
        });
        dialog.present();
    }
    fn confirm_close(&self) {
        let dialog=gtk::MessageDialog::builder().transient_for(&self.window).modal(true).text("Ungespeicherte Änderungen").secondary_text("Die App geöffnet lassen und die Änderungen nach dem Entsperren speichern. Beim Schließen wird die Wiederherstellung verworfen.").build();
        dialog.add_button("Zurück", gtk::ResponseType::Cancel);
        dialog.add_button("Verwerfen und schließen", gtk::ResponseType::Accept);
        let weak = self.weak.borrow().clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                if let Some(ui) = weak.upgrade() {
                    ui.clear_clipboard();
                    ui.application.quit();
                }
            }
            dialog.close();
        });
        dialog.present();
    }

    fn choose_local(&self) {
        if self.state.borrow().busy {
            return;
        }
        let chooser = gtk::FileChooserNative::builder()
            .transient_for(&self.window)
            .title("SafeInCloud-Datenbank öffnen")
            .action(gtk::FileChooserAction::Open)
            .accept_label("Öffnen")
            .cancel_label("Abbrechen")
            .build();
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("SafeInCloud-Datenbank (*.db)"));
        filter.add_pattern("*.db");
        chooser.add_filter(&filter);
        let weak = self.weak.borrow().clone();
        chooser.connect_response(move |chooser, response| {
            if response == gtk::ResponseType::Accept {
                if let (Some(ui), Some(path)) =
                    (weak.upgrade(), chooser.file().and_then(|f| f.path()))
                {
                    let mut s = ui.state.borrow_mut();
                    if !s.busy && !s.unlocked && s.recovery.is_none() {
                        s.path = Some(path);
                        s.session = None;
                        drop(s);
                        ui.unlock_error.set_text("");
                        ui.update_source();
                    }
                }
            }
            chooser.destroy();
        });
        chooser.show();
    }
    fn connect_drive(&self) {
        if self.state.borrow().busy || self.state.borrow().recovery.is_some() {
            return;
        }
        let drive = self.state.borrow().drive.clone();
        let Some(drive) = drive else {
            self.unlock_error
                .set_text("Drive-Konfiguration ist nicht verfügbar.");
            return;
        };
        let dialog = gtk::Dialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .title("Google Drive verbinden")
            .build();
        dialog.add_button("Abbrechen", gtk::ResponseType::Cancel);
        dialog.add_button("Verbinden", gtk::ResponseType::Accept);
        let box_ = dialog.content_area();
        box_.set_spacing(12);
        box_.set_margin_start(22);
        box_.set_margin_end(22);
        box_.set_margin_top(22);
        box_.set_margin_bottom(22);
        let info=text("Für den privaten SafeInCloud-Ordner wird die lokal vorhandene Original-Anwendung als Kompatibilitätsreferenz verwendet.","muted");
        info.set_wrap(true);
        info.set_max_width_chars(60);
        box_.append(&info);
        let path = gtk::Entry::builder()
            .placeholder_text(
                "Pfad zur offiziellen SafeInCloud.exe (optional bei bestehender Konfiguration)",
            )
            .width_chars(65)
            .build();
        box_.append(&path);
        let browse = gtk::Button::with_label("Original-Datei auswählen …");
        box_.append(&browse);
        let parent = dialog.clone();
        let entry = path.clone();
        browse.connect_clicked(move |_| {
            let chooser = gtk::FileChooserNative::builder()
                .transient_for(&parent)
                .title("Offizielle SafeInCloud.exe auswählen")
                .action(gtk::FileChooserAction::Open)
                .build();
            let entry = entry.clone();
            chooser.connect_response(move |chooser, response| {
                if response == gtk::ResponseType::Accept {
                    if let Some(file) = chooser.file().and_then(|f| f.path()) {
                        entry.set_text(&file.to_string_lossy());
                    }
                }
                chooser.destroy();
            });
            chooser.show();
        });
        let weak = self.weak.borrow().clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                if let Some(ui) = weak.upgrade() {
                    ui.state.borrow_mut().busy = true;
                    ui.unlock_error
                        .set_text("Google-Anmeldung im Browser abschließen …");
                    ui.update_source();
                    let sender = ui.sender.clone();
                    let drive = drive.clone();
                    let path = path.text().to_string();
                    std::thread::spawn(move || {
                        let result = (|| {
                            if !path.trim().is_empty() {
                                drive.set_client_executable(&PathBuf::from(path.trim()))?;
                            }
                            drive.connect()?;
                            drive.discover()
                        })()
                        .map_err(|e: anyhow::Error| e.to_string());
                        let _ = sender.send(Job::Discover(result));
                    });
                }
            }
            dialog.close();
        });
        dialog.present();
    }
    fn select_candidate(&self, candidates: Vec<Candidate>) {
        let drive = self.state.borrow().drive.clone();
        let Some(drive) = drive else {
            return;
        };
        if let Some(candidate) = drive.preferred(&candidates) {
            self.open_candidate(candidate);
            return;
        }
        if candidates.is_empty() {
            self.unlock_error
                .set_text("Keine SafeInCloud-Datenbank im privaten Google-Ordner gefunden.");
            return;
        }
        let dialog = gtk::Dialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .title("Drive-Datenbank auswählen")
            .build();
        dialog.add_button("Abbrechen", gtk::ResponseType::Cancel);
        dialog.add_button("Öffnen", gtk::ResponseType::Accept);
        let names: Vec<_> = candidates
            .iter()
            .map(|c| c.name.replace('\0', "�"))
            .collect();
        let refs: Vec<_> = names.iter().map(String::as_str).collect();
        let selection = gtk::DropDown::from_strings(&refs);
        dialog.content_area().append(&selection);
        let weak = self.weak.borrow().clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                if let (Some(ui), Some(candidate)) = (
                    weak.upgrade(),
                    candidates.get(selection.selected() as usize),
                ) {
                    ui.open_candidate(candidate.clone());
                }
            }
            dialog.close();
        });
        dialog.present();
    }
    fn open_candidate(&self, candidate: Candidate) {
        let drive = {
            let mut s = self.state.borrow_mut();
            if s.busy || s.unlocked {
                return;
            }
            s.busy = true;
            s.drive.clone()
        };
        let Some(drive) = drive else {
            return;
        };
        self.unlock_error
            .set_text("Verschlüsselte Drive-Datei wird geladen …");
        self.update_source();
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let result = drive.open_candidate(candidate).map_err(|e| e.to_string());
            let _ = sender.send(Job::Open(result));
        });
    }

    fn pump_icons(&self) {
        if !self.icons_enabled.is_active() {
            return;
        }
        loop {
            let origin = {
                let mut s = self.state.borrow_mut();
                if !s.unlocked || s.active_icons >= 4 {
                    return;
                }
                let Some(origin) = s.icon_queue.pop_front() else {
                    return;
                };
                s.active_icons += 1;
                origin
            };
            let sender = self.sender.clone();
            std::thread::spawn(move || {
                let bytes = favicons::fetch(&origin).ok();
                let _ = sender.send(Job::Icon(origin, bytes));
            });
        }
    }
    fn update_icon_widgets(&self, origin: &str) {
        let texture = self.state.borrow().icons.get(origin).cloned().flatten();
        let Some(texture) = texture else {
            return;
        };
        fn visit(widget: &gtk::Widget, name: &str, texture: &gdk::Texture) {
            if widget.widget_name() == name {
                if let Some(box_) = widget.downcast_ref::<gtk::Box>() {
                    let size = box_.width_request();
                    clear(box_);
                    let picture = gtk::Picture::for_paintable(texture);
                    picture.set_size_request(size - 8, size - 8);
                    picture.set_margin_start(4);
                    picture.set_margin_end(4);
                    picture.set_margin_top(4);
                    picture.set_margin_bottom(4);
                    box_.append(&picture);
                    box_.add_css_class("website-icon");
                }
            }
            let mut child = widget.first_child();
            while let Some(value) = child {
                visit(&value, name, texture);
                child = value.next_sibling();
            }
        }
        visit(
            self.entries.upcast_ref(),
            &format!("icon:{origin}"),
            &texture,
        );
        visit(
            self.detail.upcast_ref(),
            &format!("icon:{origin}"),
            &texture,
        );
    }

    fn screenshot(&self, path: &std::path::Path) {
        let paintable = gtk::WidgetPaintable::new(Some(&self.window));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            self.window.width() as f64,
            self.window.height() as f64,
        );
        if let (Some(node), Some(renderer)) = (snapshot.to_node(), self.window.renderer()) {
            let texture = renderer.render_texture(&node, None);
            if let Err(e) = texture.save_to_png(path) {
                eprintln!("Vorschau konnte nicht gespeichert werden: {e}");
            }
        }
    }

    fn smoke_test(&self) {
        assert!(self.state.borrow().unlocked, "Demo entsperrt");
        assert!(self.entries.row_at_index(0).is_some());
        let before_count = self.state.borrow().vault.as_ref().unwrap().cards().len();
        self.edit(false);
        let editor = self
            .application
            .windows()
            .into_iter()
            .find(|w| w.widget_name() == "editor-window")
            .expect("editor opens from selected saved record");
        let title = named(editor.upcast_ref(), "editor-title")
            .unwrap()
            .downcast::<gtk::Entry>()
            .unwrap();
        title.set_text("Geänderter Demo-Eintrag");
        let password = named(editor.upcast_ref(), "editor-password-0")
            .unwrap()
            .downcast::<gtk::Entry>()
            .unwrap();
        let apply_field = password.clone();
        let generated = dialogs::generator(
            &editor,
            self.guard(),
            self.copy_callback(),
            Some(Rc::new(move |value| apply_field.set_text(&value))),
        );
        generated.present();
        let mode = named(generated.upcast_ref(), "generator-mode")
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap();
        let output = named(generated.upcast_ref(), "generator-output")
            .unwrap()
            .downcast::<gtk::Entry>()
            .unwrap();
        for index in 0..4 {
            mode.set_selected(index);
            let value = output.text();
            assert!(!value.is_empty());
            match index {
                0 => assert_eq!(value.split('-').count(), 6),
                1 => assert!(value.chars().all(|c| c.is_ascii_alphanumeric())),
                3 => assert!(value.chars().all(|c| c.is_ascii_digit())),
                _ => {}
            }
        }
        mode.set_selected(2);
        let expected = Zeroizing::new(output.text().to_string());
        named(generated.upcast_ref(), "generator-copy")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        assert!(self.state.borrow().clipboard.is_some());
        named(generated.upcast_ref(), "generator-apply")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        assert_eq!(password.text().as_str(), expected.as_str());
        named(editor.upcast_ref(), "editor-save")
            .unwrap()
            .downcast::<gtk::Button>()
            .unwrap()
            .emit_clicked();
        let path = self.state.borrow().path.clone().unwrap();
        let reopened = Vault::open(&path, "synthetic-demo-only").unwrap();
        assert_eq!(reopened.cards().len(), before_count);
        let changed = reopened
            .cards()
            .into_iter()
            .find(|c| c.title == "Geänderter Demo-Eintrag")
            .expect("edited title survives reopen");
        assert!(changed
            .fields
            .iter()
            .any(|f| f.kind == "password" && f.value == *expected));
        let dialog = dialogs::generator(
            self.window.upcast_ref(),
            self.guard(),
            self.copy_callback(),
            None,
        );
        dialog.present();
        self.copy_value("synthetic-clipboard-only");
        assert!(self.state.borrow().clipboard.is_some());
        let stale_guard = self.guard();
        self.lock();
        assert!(!stale_guard());
        assert!(self.state.borrow().vault.is_none());
        assert!(self.state.borrow().clipboard.is_none());
        assert_eq!(self.stack.visible_child_name().as_deref(), Some("locked"));
        println!("GTK smoke OK: unlock, select, edit, encrypted save/reopen, generator, clipboard and lock.");
    }
}

fn named(root: &gtk::Widget, name: &str) -> Option<gtk::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = named(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn set_editable(root: &gtk::Widget, enabled: bool) {
    if root.has_css_class("requires-edit") {
        root.set_sensitive(enabled);
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        set_editable(&widget, enabled);
        child = widget.next_sibling();
    }
}

fn load_icons_preference() -> bool {
    oauth::config_dir()
        .ok()
        .and_then(|dir| std::fs::read(dir.join("preferences.json")).ok())
        .and_then(|raw| serde_json::from_slice::<serde_json::Value>(&raw).ok())
        .and_then(|value| value.get("website_icons").and_then(|v| v.as_bool()))
        .unwrap_or(true)
}
fn save_icons_preference(enabled: bool) {
    if let Ok(dir) = oauth::config_dir() {
        if std::fs::create_dir_all(&dir).is_ok() {
            let path = dir.join("preferences.json");
            let _ = std::fs::write(path, format!("{{\"website_icons\":{enabled}}}"));
        }
    }
}

pub fn run(args: Arguments) {
    fonts::register();
    let application = gtk::Application::builder()
        .application_id("io.github.vassiljevnikita.SafeInCloudLinux")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let args = RefCell::new(Some(args));
    let owner: Rc<RefCell<Option<Rc<Ui>>>> = Rc::new(RefCell::new(None));
    let hold = owner.clone();
    application.connect_activate(move |application| {
        if let Some(args) = args.borrow_mut().take() {
            let provider = gtk::CssProvider::new();
            provider.load_from_data(include_str!("../../assets/style.css"));
            gtk::style_context_add_provider_for_display(
                &gdk::Display::default().expect("GTK display"),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            hold.replace(Some(Ui::new(application, args)));
        } else if let Some(ui) = hold.borrow().as_ref() {
            ui.window.present();
        }
    });
    application.run_with_args::<&str>(&[]);
    owner.borrow_mut().take();
}
