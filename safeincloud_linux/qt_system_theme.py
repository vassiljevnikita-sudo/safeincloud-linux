"""Follow Qt and the Linux desktop's read-only appearance setting, without polling."""
import sys
from string import Template
from weakref import WeakKeyDictionary

from PySide6.QtCore import Qt, QObject, Signal, Slot, SLOT
from PySide6.QtGui import QPalette, QColor
from PySide6.QtWidgets import QApplication
from PySide6.QtDBus import QDBus, QDBusConnection, QDBusMessage, QDBusVariant
from shiboken6 import isValid


LIGHT = dict(
    ink='#2f2f35', window='#fafafb', sidebar='#ebebed', list='#f3f3f5',
    surface='#ffffff', border='#d5d5d8', divider='#e3e3e6', muted='#5e5e66',
    search='#e4e4e7', button='#e8e8eb', button_hover='#dddde1', button_pressed='#cfcfd5',
    disabled='#94949b', disabled_bg='#eeeef0', accent='#3c6eb4', accent_ink='#335f9e',
    primary='#3c6eb4', primary_hover='#335f9e', primary_text='#ffffff', primary_disabled='#a1b5d2',
    danger='#a8202b', danger_bg='#fbe4e5', input_border='#d8d8de', selection='#cbd9ed',
    surface_border='#dedee3', row_border='#ededf0', focus_bg='#edf2fa', avatar_bg='#d8e6f5',
    scrollbar='#c7c7ce', switch_off='#b7b7c0', switch_bg='#eeeef1',
    entry_selected='#d8e0ed', entry_hover='#e9e9ed', nav_selected='#d6d6dc', nav_hover='#e1e1e6',
    avatars=(('#d2efdf', '#216a46'), ('#ffdfc5', '#974c11'), ('#dae7ef', '#246477'), ('#e7d9f6', '#66408c')),
)
DARK = dict(
    ink='#f3f3f6', window='#202024', sidebar='#27272c', list='#232327',
    surface='#2c2c31', border='#424248', divider='#3c3c43', muted='#b5b5bf',
    search='#35353c', button='#37373e', button_hover='#45454d', button_pressed='#50505b',
    disabled='#8d8d96', disabled_bg='#2f2f35', accent='#91bbf5', accent_ink='#a8cbff',
    primary='#3c6eb4', primary_hover='#3d6ba6', primary_text='#ffffff', primary_disabled='#34455c',
    danger='#ffb4b8', danger_bg='#4a292f', input_border='#50505a', selection='#3f5270',
    surface_border='#48484f', row_border='#414148', focus_bg='#303e53', avatar_bg='#2c425e',
    scrollbar='#62626e', switch_off='#777781', switch_bg='#3a3a42',
    entry_selected='#34465e', entry_hover='#34343c', nav_selected='#42424c', nav_hover='#35353e',
    avatars=(('#254738', '#b8ead0'), ('#513c29', '#ffd5ab'), ('#263f49', '#b9e3ee'), ('#443452', '#e5cafa')),
)


def system_theme():
    app = QApplication.instance()
    if not hasattr(app, '_safe_system_theme'):
        app._safe_system_theme = SystemTheme(app)
    return app._safe_system_theme


def theme_colors():
    return system_theme().colors


def bind_icon(target, name, tone='muted', size=None):
    """Recolor existing buttons, actions and brand marks during live switching."""
    theme = system_theme()
    theme.icons[target] = (name, tone, size)
    theme.refresh_icon(target, (name, tone, size))


class SystemTheme(QObject):
    changed = Signal(bool)

    def __init__(self, app):
        super().__init__(app)
        self.app = app
        self.icons = WeakKeyDictionary()
        self._qt_scheme = app.styleHints().colorScheme()
        self._portal_scheme = 0
        self._fallback_dark = app.palette().color(QPalette.Window).lightness() < 128
        self.dark = self._qt_scheme == Qt.ColorScheme.Dark or (
            self._qt_scheme == Qt.ColorScheme.Unknown and self._fallback_dark)
        self.colors = DARK if self.dark else LIGHT
        app.styleHints().colorSchemeChanged.connect(self.qt_scheme_changed)
        self._watch_portal()
        self.apply()

    def _watch_portal(self):
        if sys.platform != 'linux':
            return
        self._bus = QDBusConnection.sessionBus()
        if not self._bus.isConnected():
            return
        self._portal_connected = self._bus.connect(
            'org.freedesktop.portal.Desktop', '/org/freedesktop/portal/desktop',
            'org.freedesktop.portal.Settings', 'SettingChanged', self,
            SLOT('portal_setting_changed(QString,QString,QDBusVariant)'))
        message = QDBusMessage.createMethodCall(
            'org.freedesktop.portal.Desktop', '/org/freedesktop/portal/desktop',
            'org.freedesktop.portal.Settings', 'Read')
        message.setArguments(['org.freedesktop.appearance', 'color-scheme'])
        reply = self._bus.call(message, QDBus.Block, 500)
        if reply.type() == QDBusMessage.ReplyMessage and reply.arguments():
            self._portal_scheme = self._portal_value(reply.arguments()[0])
            if self._portal_scheme in (1, 2):
                self.dark = self._portal_scheme == 1
                self.colors = DARK if self.dark else LIGHT

    @staticmethod
    def _portal_value(value):
        while isinstance(value, QDBusVariant):
            value = value.variant()
        return value if type(value) is int and value in (1, 2) else 0

    @Slot(str, str, QDBusVariant)
    def portal_setting_changed(self, namespace, key, value):
        if namespace != 'org.freedesktop.appearance' or key != 'color-scheme':
            return
        self._portal_scheme = self._portal_value(value)
        if self._portal_scheme in (1, 2):
            self.set_dark(self._portal_scheme == 1)
        else:
            self.qt_scheme_changed(self.app.styleHints().colorScheme())

    @Slot(Qt.ColorScheme)
    def qt_scheme_changed(self, scheme):
        self._qt_scheme = scheme
        if scheme != Qt.ColorScheme.Unknown:
            self.set_dark(scheme == Qt.ColorScheme.Dark)
        elif self._portal_scheme in (1, 2):
            self.set_dark(self._portal_scheme == 1)
        else:
            self.set_dark(self._fallback_dark)

    def set_dark(self, dark):
        if self.dark != dark:
            self.dark = dark
            self.colors = DARK if dark else LIGHT
            self.apply()
            self.changed.emit(dark)

    def refresh_icon(self, target, spec):
        from .qt_theme import icon
        name, tone, size = spec
        value = icon(name, self.colors[tone])
        if size is None:
            target.setIcon(value)
        else:
            target.setPixmap(value.pixmap(size, size))

    def apply(self):
        from .qt_theme import STYLE_TEMPLATE
        colors = self.colors
        palette = QPalette(self.app.palette())
        roles = {QPalette.Window: 'window', QPalette.WindowText: 'ink', QPalette.Base: 'surface',
                 QPalette.AlternateBase: 'list', QPalette.Text: 'ink', QPalette.Button: 'button',
                 QPalette.ButtonText: 'ink', QPalette.ToolTipBase: 'surface', QPalette.ToolTipText: 'ink',
                 QPalette.Highlight: 'primary', QPalette.HighlightedText: 'primary_text',
                 QPalette.Link: 'accent', QPalette.PlaceholderText: 'muted', QPalette.BrightText: 'danger'}
        for role, token in roles.items():
            palette.setColor(role, QColor(colors[token]))
        for role in (QPalette.WindowText, QPalette.Text, QPalette.ButtonText):
            palette.setColor(QPalette.Disabled, role, QColor(colors['disabled']))
        self.app.setPalette(palette)
        self.app.setStyleSheet(Template(STYLE_TEMPLATE).substitute(colors))
        for target, spec in list(self.icons.items()):
            if isValid(target):
                self.refresh_icon(target, spec)
            else:
                del self.icons[target]
        for widget in self.app.allWidgets():
            widget.update()
