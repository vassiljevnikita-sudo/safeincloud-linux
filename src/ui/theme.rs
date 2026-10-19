//! Follow the desktop appearance portal without changing desktop preferences.
use gtk::{gio, glib, prelude::*};
use std::{cell::Cell, rc::Rc};

const APPEARANCE: &str = "org.freedesktop.appearance";
const COLOR_SCHEME: &str = "color-scheme";

/// Read replies wrap their value in a tuple and a variant. Some portal backends
/// introduce another variant wrapper; accept those without assuming a depth.
fn scheme(value: &glib::Variant) -> Option<u32> {
    let mut value = value.clone();
    for _ in 0..16 {
        if let Some(value) = value.get::<u32>() {
            return (value <= 2).then_some(value);
        }
        if value.type_() == glib::VariantTy::VARIANT {
            value = value.as_variant()?;
        } else if value.type_().is_tuple() && value.n_children() == 1 {
            value = value.child_value(0);
        } else {
            return None;
        }
    }
    None
}

fn changed_scheme(parameters: &glib::Variant) -> Option<u32> {
    if parameters.type_().as_str() != "(ssv)" {
        return None;
    }
    let (namespace, key, value) = parameters.get::<(String, String, glib::Variant)>()?;
    if namespace == APPEARANCE && key == COLOR_SCHEME {
        scheme(&value)
    } else {
        None
    }
}

fn apply(scheme: u32, native_dark: bool) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(match scheme {
            1 => true,
            2 => false,
            _ => native_dark,
        });
    }
}

fn read(proxy: &gio::DBusProxy, native_dark: bool, revision: Rc<Cell<u64>>) {
    let requested_revision = revision.get();
    proxy.call(
        "Read",
        Some(&(APPEARANCE, COLOR_SCHEME).to_variant()),
        gio::DBusCallFlags::NO_AUTO_START,
        2_000,
        gio::Cancellable::NONE,
        move |result| {
            // A SettingChanged signal or a portal restart supersedes this read.
            if revision.get() != requested_revision {
                return;
            }
            let preference = result.ok().and_then(|reply| scheme(&reply)).unwrap_or(0);
            apply(preference, native_dark);
        },
    );
}

/// Bind before constructing windows; retain the returned proxy for live updates.
/// Missing portals and unsupported settings quietly retain the native GTK theme.
pub fn bind() -> Option<gio::DBusProxy> {
    let native_dark = gtk::Settings::default()?.is_gtk_application_prefer_dark_theme();
    let proxy = gio::DBusProxy::for_bus_sync(
        gio::BusType::Session,
        gio::DBusProxyFlags::DO_NOT_AUTO_START | gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES,
        None,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Settings",
        gio::Cancellable::NONE,
    )
    .ok()?;
    let revision = Rc::new(Cell::new(0_u64));
    let signal_revision = revision.clone();
    // The local connector preserves GTK's main-thread requirement and works
    // across GIO versions with either detailed or undetailed g-signal bindings.
    proxy.connect_local("g-signal", false, move |values| {
        let signal = values.get(2).and_then(|value| value.get::<String>().ok());
        if signal.as_deref() == Some("SettingChanged") {
            if let Some(preference) = values
                .get(3)
                .and_then(|value| value.get::<glib::Variant>().ok())
                .and_then(|parameters| changed_scheme(&parameters))
            {
                signal_revision.set(signal_revision.get().wrapping_add(1));
                apply(preference, native_dark);
            }
        }
        None
    });
    let owner_revision = revision.clone();
    proxy.connect_notify_local(Some("g-name-owner"), move |proxy, _| {
        owner_revision.set(owner_revision.get().wrapping_add(1));
        if proxy.name_owner().is_some() {
            read(proxy, native_dark, owner_revision.clone());
        } else {
            apply(0, native_dark);
        }
    });
    read(&proxy, native_dark, revision);
    Some(proxy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_portal_reply_wrappers_and_rejects_invalid_values() {
        for preference in 0_u32..=2 {
            let value = preference.to_variant();
            assert_eq!(scheme(&value), Some(preference));
            let boxed = glib::Variant::from_variant(&value);
            let nested = glib::Variant::from_variant(&boxed);
            assert_eq!(scheme(&(nested,).to_variant()), Some(preference));
        }
        assert_eq!(scheme(&3_u32.to_variant()), None);
        assert_eq!(scheme(&"dark".to_variant()), None);
        assert_eq!(scheme(&(1_u32, 2_u32).to_variant()), None);
    }

    #[test]
    fn live_events_require_the_appearance_namespace_and_color_scheme_key() {
        let value = glib::Variant::from_variant(&1_u32.to_variant());
        assert_eq!(
            changed_scheme(&(APPEARANCE, COLOR_SCHEME, value.clone()).to_variant()),
            Some(1)
        );
        assert_eq!(
            changed_scheme(&("other.namespace", COLOR_SCHEME, value.clone()).to_variant()),
            None
        );
        assert_eq!(
            changed_scheme(&(APPEARANCE, "contrast", value).to_variant()),
            None
        );
        assert_eq!(changed_scheme(&().to_variant()), None);
        assert_eq!(
            changed_scheme(&(APPEARANCE, COLOR_SCHEME, 1_u32).to_variant()),
            None
        );
    }
}
