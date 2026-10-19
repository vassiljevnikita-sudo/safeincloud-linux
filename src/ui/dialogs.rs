//! Asynchronous, session-guarded native credential dialogs.
use crate::{
    passwords::{self, Mode, Options},
    vault::{Card, Field},
};
use gtk::{glib, prelude::*};
use std::rc::Rc;
use zeroize::Zeroize;

pub type Guard = Rc<dyn Fn() -> bool>;
pub type CopyCallback = Rc<dyn Fn(&str)>;
pub type SaveCallback = Rc<dyn Fn(String, Vec<Field>, String) -> bool>;

fn caption(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.add_css_class("section-title");
    label
}

fn margins(widget: &impl IsA<gtk::Widget>, amount: i32) {
    widget.set_margin_top(amount);
    widget.set_margin_bottom(amount);
    widget.set_margin_start(amount);
    widget.set_margin_end(amount);
}

fn secret(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("password") || kind.eq_ignore_ascii_case("secret")
}

// Keyboard and context-menu copies use the same expiring clipboard as the
// explicit Copy button. GtkEntry delegates these action signals to GtkText.
fn route_secret_clipboard(entry: &gtk::Entry, guard: Guard, on_copy: CopyCallback) {
    if let Some(text) = entry.delegate().and_downcast::<gtk::Text>() {
        let copy_guard = guard.clone();
        let copy = on_copy.clone();
        text.connect_copy_clipboard(move |text| {
            text.stop_signal_emission_by_name("copy-clipboard");
            if !copy_guard() || !text.property::<bool>("visibility") {
                return;
            }
            if let Some((start, end)) = text.selection_bounds() {
                let mut selected: String = text
                    .text()
                    .chars()
                    .skip(start.min(end) as usize)
                    .take((end - start).unsigned_abs() as usize)
                    .collect();
                if !selected.is_empty() {
                    copy(&selected);
                }
                selected.zeroize();
            }
        });
        text.connect_cut_clipboard(move |text| {
            text.stop_signal_emission_by_name("cut-clipboard");
            if !guard() || !text.is_editable() || !text.property::<bool>("visibility") {
                return;
            }
            if let Some((start, end)) = text.selection_bounds() {
                let mut selected: String = text
                    .text()
                    .chars()
                    .skip(start.min(end) as usize)
                    .take((end - start).unsigned_abs() as usize)
                    .collect();
                if !selected.is_empty() {
                    on_copy(&selected);
                    text.delete_text(start.min(end), start.max(end));
                }
                selected.zeroize();
            }
        });
    }
}

fn update_strength(widget: &gtk::Box, value: &str, context: &[&str]) {
    let result = passwords::assess(value, context);
    if let Some(label) = widget.first_child().and_downcast::<gtk::Label>() {
        label.set_text(&if result.estimate.is_empty() {
            result.label.clone()
        } else {
            format!("{} · {}", result.label, result.estimate)
        });
        label.set_wrap(true);
    }
    if let Some(bar) = widget.last_child().and_downcast::<gtk::ProgressBar>() {
        bar.set_fraction(result.score.map_or(0.0, |score| (score as f64 + 1.0) / 5.0));
        for class in ["error", "warning", "success"] {
            bar.remove_css_class(class);
        }
        if let Some(score) = result.score {
            bar.add_css_class(if score < 2 {
                "error"
            } else if score < 3 {
                "warning"
            } else {
                "success"
            });
        }
    }
    let tooltip = if result.estimate.is_empty() {
        "Lokale Bewertung mit zxcvbn. Modell: 10.000 Versuche pro Sekunde; keine Garantie.".into()
    } else {
        format!(
            "{}\nLokale Schätzung mit zxcvbn bei 10.000 Versuchen pro Sekunde; keine Garantie.",
            result.estimate
        )
    };
    widget.set_tooltip_text(Some(&tooltip));
}

/// A local zxcvbn summary. The tooltip states the estimate's attack-rate model.
pub fn strength_widget(value: &str, context: &[&str]) -> gtk::Box {
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 5);
    let label = gtk::Label::new(None);
    label.set_xalign(0.0);
    label.add_css_class("muted");
    let progress = gtk::ProgressBar::new();
    progress.set_hexpand(true);
    widget.append(&label);
    widget.append(&progress);
    update_strength(&widget, value, context);
    widget
}

/// Build without presenting, so the owner can register the window before use.
pub fn editor(
    parent: &gtk::Window,
    card: Option<Card>,
    guard: Guard,
    on_save: SaveCallback,
    on_copy: CopyCallback,
) -> gtk::Window {
    let existing = card.is_some();
    let window = gtk::Window::builder()
        .title(if existing {
            "Eintrag bearbeiten"
        } else {
            "Neuer Eintrag"
        })
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .default_width(560)
        .default_height(680)
        .build();
    if let Some(application) = parent.application() {
        window.set_application(Some(&application));
    }
    window.set_widget_name("editor-window");
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
    margins(&body, 24);
    let heading = gtk::Label::new(Some(if existing {
        "Eintrag bearbeiten"
    } else {
        "Neuer Eintrag"
    }));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    body.append(&heading);
    body.append(&caption("Titel"));
    let title = gtk::Entry::new();
    title.set_widget_name("editor-title");
    title.set_placeholder_text(Some("Zum Beispiel: E-Mail oder Bank"));
    body.append(&title);
    let (mut initial_title, initial_fields, mut initial_notes) = match card {
        Some(card) => (card.title, card.fields, card.notes),
        None => (
            String::new(),
            vec![
                Field {
                    name: "Login".into(),
                    kind: "login".into(),
                    value: String::new(),
                },
                Field {
                    name: "Passwort".into(),
                    kind: "password".into(),
                    value: String::new(),
                },
                Field {
                    name: "Website".into(),
                    kind: "website".into(),
                    value: String::new(),
                },
            ],
            String::new(),
        ),
    };
    title.set_text(&initial_title);
    initial_title.zeroize();
    let mut fields = Vec::new();
    let mut strengths = Vec::new();
    let mut password_index = 0;
    for mut field in initial_fields {
        body.append(&caption(&field.name));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let input = gtk::Entry::new();
        input.set_hexpand(true);
        input.set_text(&field.value);
        field.value.zeroize();
        row.append(&input);
        if secret(&field.kind) {
            input.set_widget_name(&format!("editor-password-{password_index}"));
            password_index += 1;
            input.set_visibility(false);
            input.set_input_purpose(gtk::InputPurpose::Password);
            route_secret_clipboard(&input, guard.clone(), on_copy.clone());
            let reveal = gtk::ToggleButton::with_label("Anzeigen");
            reveal.set_tooltip_text(Some("Passwort anzeigen oder verbergen"));
            let input_weak = input.downgrade();
            let reveal_guard = guard.clone();
            reveal.connect_toggled(move |button| {
                if let Some(input) = input_weak.upgrade() {
                    input.set_visibility(reveal_guard() && button.is_active());
                    button.set_label(if input.property::<bool>("visibility") {
                        "Verbergen"
                    } else {
                        "Anzeigen"
                    });
                }
            });
            row.append(&reveal);
            let generate = gtk::Button::with_label("Generieren");
            generate.set_tooltip_text(Some("Passwortgenerator öffnen"));
            let input_weak = input.downgrade();
            let window_weak = window.downgrade();
            let guard = guard.clone();
            let copy = on_copy.clone();
            generate.connect_clicked(move |_| {
                if !guard() {
                    return;
                }
                if let Some(parent) = window_weak.upgrade() {
                    let input_weak = input_weak.clone();
                    let apply_guard = guard.clone();
                    let apply: Rc<dyn Fn(String)> = Rc::new(move |mut value| {
                        if apply_guard() {
                            if let Some(input) = input_weak.upgrade() {
                                input.set_text(&value);
                            }
                        }
                        value.zeroize();
                    });
                    generator(&parent, guard.clone(), copy.clone(), Some(apply)).present();
                }
            });
            row.append(&generate);
            body.append(&row);
            let strength = strength_widget(&input.text(), &[]);
            body.append(&strength);
            strengths.push((input.downgrade(), strength.downgrade()));
        } else {
            input.set_widget_name(&format!("editor-field-{}", fields.len()));
            body.append(&row);
        }
        fields.push((field.name, field.kind, input));
    }
    let title_weak = title.downgrade();
    let context_entries: Vec<_> = fields
        .iter()
        .filter(|(name, kind, _)| {
            let name = name.to_lowercase();
            let kind = kind.to_lowercase();
            !secret(&kind)
                && (kind == "login"
                    || kind == "email"
                    || name.contains("login")
                    || name.contains("benutzer")
                    || name.contains("mail"))
        })
        .map(|(_, _, entry)| entry.downgrade())
        .collect();
    let strength_guard = guard.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        if !strength_guard() {
            return;
        }
        let mut context = vec![title_weak
            .upgrade()
            .map(|e| e.text().to_string())
            .unwrap_or_default()];
        context.extend(
            context_entries
                .iter()
                .filter_map(|weak| weak.upgrade().map(|entry| entry.text().to_string())),
        );
        let refs: Vec<&str> = context.iter().map(String::as_str).collect();
        for (entry, strength) in &strengths {
            if let (Some(entry), Some(strength)) = (entry.upgrade(), strength.upgrade()) {
                update_strength(&strength, &entry.text(), &refs);
            }
        }
        for item in &mut context {
            item.zeroize();
        }
    });
    let refresh_title = refresh.clone();
    title.connect_changed(move |_| refresh_title());
    for (_, _, input) in &fields {
        let refresh = refresh.clone();
        input.connect_changed(move |_| refresh());
    }
    refresh();
    body.append(&caption("Notizen"));
    let notes = gtk::TextView::new();
    notes.set_widget_name("editor-notes");
    notes.set_wrap_mode(gtk::WrapMode::WordChar);
    notes.set_top_margin(10);
    notes.set_bottom_margin(10);
    notes.set_left_margin(10);
    notes.set_right_margin(10);
    notes.buffer().set_text(&initial_notes);
    initial_notes.zeroize();
    let notes_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(110)
        .child(&notes)
        .build();
    notes_scroll.add_css_class("card");
    body.append(&notes_scroll);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&body)
        .build();
    outer.append(&scroll);
    outer.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    margins(&actions, 16);
    actions.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Abbrechen");
    let save = gtk::Button::with_label("Speichern");
    save.set_widget_name("editor-save");
    save.add_css_class("suggested-action");
    save.set_sensitive(!title.text().trim().is_empty());
    let save_weak = save.downgrade();
    title.connect_changed(move |entry| {
        if let Some(save) = save_weak.upgrade() {
            save.set_sensitive(!entry.text().trim().is_empty());
        }
    });
    let window_weak = window.downgrade();
    cancel.connect_clicked(move |_| {
        if let Some(window) = window_weak.upgrade() {
            window.close();
        }
    });
    let window_weak = window.downgrade();
    let fields = Rc::new(fields);
    let save_fields = fields.clone();
    let save_title = title.clone();
    let notes_buffer = notes.buffer();
    let save_notes = notes_buffer.clone();
    save.connect_clicked(move |_| {
        if !guard() || save_title.text().trim().is_empty() {
            return;
        }
        let values = save_fields
            .iter()
            .map(|(name, kind, entry)| Field {
                name: name.clone(),
                kind: kind.clone(),
                value: entry.text().to_string(),
            })
            .collect();
        let notes = save_notes
            .text(&save_notes.start_iter(), &save_notes.end_iter(), true)
            .to_string();
        if on_save(save_title.text().to_string(), values, notes) {
            if let Some(window) = window_weak.upgrade() {
                window.close();
            }
        }
    });
    window.connect_destroy(move |_| {
        title.set_text("");
        for (_, _, entry) in fields.iter() {
            entry.set_text("");
        }
        notes_buffer.set_text("");
    });
    actions.append(&cancel);
    actions.append(&save);
    outer.append(&actions);
    window.set_child(Some(&outer));
    window
}

#[derive(Clone)]
struct GeneratorControls {
    mode: glib::WeakRef<gtk::DropDown>,
    length: glib::WeakRef<gtk::Adjustment>,
    lower: glib::WeakRef<gtk::CheckButton>,
    upper: glib::WeakRef<gtk::CheckButton>,
    digits: glib::WeakRef<gtk::CheckButton>,
    symbols: glib::WeakRef<gtk::CheckButton>,
    ambiguous: glib::WeakRef<gtk::CheckButton>,
    output: glib::WeakRef<gtk::Entry>,
    error: glib::WeakRef<gtk::Label>,
    strength: glib::WeakRef<gtk::Box>,
    copy: glib::WeakRef<gtk::Button>,
    apply: glib::WeakRef<gtk::Button>,
}

/// Build a local generator. Applying is explicit and guarded by the vault session.
pub fn generator(
    parent: &gtk::Window,
    guard: Guard,
    on_copy: CopyCallback,
    on_apply: Option<Rc<dyn Fn(String)>>,
) -> gtk::Window {
    let window = gtk::Window::builder()
        .title("Passwortgenerator")
        .transient_for(parent)
        .modal(true)
        .destroy_with_parent(true)
        .default_width(560)
        .resizable(false)
        .build();
    if let Some(application) = parent.application() {
        window.set_application(Some(&application));
    }
    window.set_widget_name("generator-window");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 18);
    margins(&body, 24);
    let heading = gtk::Label::new(Some("Passwortgenerator"));
    heading.set_xalign(0.0);
    heading.add_css_class("heading");
    body.append(&heading);
    body.append(&caption("Passwort-Typ"));
    let mode = gtk::DropDown::from_strings(&[
        "Unvergesslich",
        "Buchstaben und Zahlen",
        "Zufällig",
        "Nur Zahlen",
    ]);
    mode.set_widget_name("generator-mode");
    mode.set_selected(2);
    body.append(&mode);
    let length_title = caption("Länge");
    body.append(&length_title);
    let length = gtk::Adjustment::new(20.0, 4.0, 64.0, 1.0, 4.0, 0.0);
    let lengths = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let slider = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&length));
    slider.set_widget_name("generator-length-slider");
    slider.set_draw_value(false);
    slider.set_hexpand(true);
    let spin = gtk::SpinButton::new(Some(&length), 1.0, 0);
    spin.set_widget_name("generator-length");
    lengths.append(&slider);
    lengths.append(&spin);
    body.append(&lengths);
    let classes = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let lower = gtk::CheckButton::with_label("Kleinbuchstaben (a–z)");
    let upper = gtk::CheckButton::with_label("Großbuchstaben (A–Z)");
    let digits = gtk::CheckButton::with_label("Zahlen (0–9)");
    let symbols = gtk::CheckButton::with_label("Sonderzeichen (!@#…)");
    for (index, button) in [&lower, &upper, &digits, &symbols].into_iter().enumerate() {
        button.set_active(true);
        button.set_widget_name(&format!("generator-class-{index}"));
        classes.append(button);
    }
    body.append(&classes);
    let ambiguous =
        gtk::CheckButton::with_label("Ähnliche Zeichen ausschließen (I, l, 1, O, 0, o, |)");
    ambiguous.set_active(true);
    ambiguous.set_widget_name("generator-exclude-ambiguous");
    body.append(&ambiguous);
    let output = gtk::Entry::new();
    output.set_widget_name("generator-output");
    output.set_editable(false);
    output.add_css_class("secret");
    route_secret_clipboard(&output, guard.clone(), on_copy.clone());
    output.set_tooltip_text(Some(
        "Lokal mit der sicheren Zufallsquelle des Betriebssystems erzeugt",
    ));
    body.append(&output);
    let strength = strength_widget("", &[]);
    body.append(&strength);
    let error = gtk::Label::new(None);
    error.set_xalign(0.0);
    error.set_wrap(true);
    error.add_css_class("error");
    error.set_visible(false);
    body.append(&error);
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let regenerate = gtk::Button::with_label("Neu generieren");
    regenerate.set_widget_name("generator-regenerate");
    let copy = gtk::Button::with_label("Kopieren");
    copy.set_widget_name("generator-copy");
    let close = gtk::Button::with_label(if on_apply.is_some() {
        "Abbrechen"
    } else {
        "Schließen"
    });
    let apply = gtk::Button::with_label("Übernehmen");
    apply.set_widget_name("generator-apply");
    apply.add_css_class("suggested-action");
    actions.append(&regenerate);
    actions.append(&copy);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    actions.append(&spacer);
    actions.append(&close);
    apply.set_visible(on_apply.is_some());
    actions.append(&apply);
    body.append(&actions);
    let controls = GeneratorControls {
        mode: mode.downgrade(),
        length: length.downgrade(),
        lower: lower.downgrade(),
        upper: upper.downgrade(),
        digits: digits.downgrade(),
        symbols: symbols.downgrade(),
        ambiguous: ambiguous.downgrade(),
        output: output.downgrade(),
        error: error.downgrade(),
        strength: strength.downgrade(),
        copy: copy.downgrade(),
        apply: apply.downgrade(),
    };
    let refresh_guard = guard.clone();
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let c = &controls;
        let (
            Some(mode),
            Some(length),
            Some(lower),
            Some(upper),
            Some(digits),
            Some(symbols),
            Some(ambiguous),
            Some(output),
            Some(error),
            Some(strength),
            Some(copy),
            Some(apply),
        ) = (
            c.mode.upgrade(),
            c.length.upgrade(),
            c.lower.upgrade(),
            c.upper.upgrade(),
            c.digits.upgrade(),
            c.symbols.upgrade(),
            c.ambiguous.upgrade(),
            c.output.upgrade(),
            c.error.upgrade(),
            c.strength.upgrade(),
            c.copy.upgrade(),
            c.apply.upgrade(),
        )
        else {
            return;
        };
        if !refresh_guard() {
            output.set_text("");
            copy.set_sensitive(false);
            apply.set_sensitive(false);
            return;
        }
        let options = Options {
            mode: match mode.selected() {
                0 => Mode::Memorable,
                1 => Mode::Alphanumeric,
                3 => Mode::Digits,
                _ => Mode::Random,
            },
            length: length.value().round() as usize,
            lower: lower.is_active(),
            upper: upper.is_active(),
            digits: digits.is_active(),
            symbols: symbols.is_active(),
            exclude_ambiguous: ambiguous.is_active(),
        };
        match passwords::generate(&options) {
            Ok(mut value) => {
                output.set_text(&value);
                update_strength(&strength, &value, &[]);
                value.zeroize();
                error.set_visible(false);
                copy.set_sensitive(true);
                apply.set_sensitive(true);
            }
            Err(problem) => {
                output.set_text("");
                update_strength(&strength, "", &[]);
                error.set_text(&problem.to_string());
                error.set_visible(true);
                copy.set_sensitive(false);
                apply.set_sensitive(false);
            }
        }
    });
    let change_refresh = refresh.clone();
    let weak_length = length.downgrade();
    let weak_classes = classes.downgrade();
    let weak_ambiguous = ambiguous.downgrade();
    let weak_title = length_title.downgrade();
    mode.connect_selected_notify(move |mode| {
        let memorable = mode.selected() == 0;
        if let Some(classes) = weak_classes.upgrade() {
            classes.set_visible(mode.selected() == 2);
        }
        if let Some(ambiguous) = weak_ambiguous.upgrade() {
            ambiguous.set_sensitive(mode.selected() == 1 || mode.selected() == 2);
        }
        if let Some(title) = weak_title.upgrade() {
            title.set_text(if memorable {
                "Anzahl der Wörter"
            } else {
                "Länge"
            });
        }
        if let Some(length) = weak_length.upgrade() {
            length.configure(
                if memorable { 6.0 } else { 20.0 },
                4.0,
                if memorable { 10.0 } else { 64.0 },
                1.0,
                if memorable { 1.0 } else { 4.0 },
                0.0,
            );
        }
        change_refresh();
    });
    let length_refresh = refresh.clone();
    length.connect_value_changed(move |_| length_refresh());
    for check in [&lower, &upper, &digits, &symbols, &ambiguous] {
        let refresh = refresh.clone();
        check.connect_toggled(move |_| refresh());
    }
    let regenerate_refresh = refresh.clone();
    regenerate.connect_clicked(move |_| regenerate_refresh());
    let weak_output = output.downgrade();
    let copy_guard = guard.clone();
    copy.connect_clicked(move |_| {
        if !copy_guard() {
            return;
        }
        if let Some(output) = weak_output.upgrade() {
            let mut text = output.text().to_string();
            if !text.is_empty() {
                on_copy(&text);
            }
            text.zeroize();
        }
    });
    let weak_window = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = weak_window.upgrade() {
            window.close();
        }
    });
    let weak_window = window.downgrade();
    let weak_output = output.downgrade();
    apply.connect_clicked(move |_| {
        if !guard() {
            return;
        }
        if let (Some(output), Some(callback)) = (weak_output.upgrade(), &on_apply) {
            let text = output.text().to_string();
            if !text.is_empty() {
                callback(text);
                if let Some(window) = weak_window.upgrade() {
                    window.close();
                }
            }
        }
    });
    window.connect_destroy(move |_| output.set_text(""));
    window.set_child(Some(&body));
    refresh();
    window
}
