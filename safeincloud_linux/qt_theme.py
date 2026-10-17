"""Adwaita layout palette with the selected Register typography, bundled offline."""
from pathlib import Path

from PySide6.QtCore import Qt, QByteArray
from PySide6.QtGui import QFontDatabase, QIcon, QPainter, QPixmap
from PySide6.QtSvg import QSvgRenderer
from .qt_system_theme import theme_colors


def load_fonts():
    """Register application fonts once per QApplication, including installed wheels."""
    from PySide6.QtWidgets import QApplication
    app = QApplication.instance()
    if app is not None and not app.property('safeFontsLoaded'):
        for path in sorted((Path(__file__).parent / 'assets' / 'fonts').glob('*.ttf')):
            QFontDatabase.addApplicationFont(str(path))
        app.setProperty('safeFontsLoaded', True)


_PATHS = {
    'search': '<circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4 4"/>',
    'plus': '<path d="M12 5v14M5 12h14"/>',
    'lock': '<rect x="5" y="10" width="14" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>',
    'edit': '<path d="m16 3 5 5-12 12-6 1 1-6Z M13 6l5 5"/>',
    'copy': '<rect x="8" y="8" width="12" height="13" rx="2"/><path d="M16 8V3H3v13h5"/>',
    'eye': '<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Z"/><circle cx="12" cy="12" r="3"/>',
    'hide': '<path d="m3 3 18 18M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Z"/>',
    'sync': '<path d="M20 7v5h-5M4 17v-5h5M5 8a8 8 0 0 1 13-3l2 3M4 16l2 3a8 8 0 0 0 13-3"/>',
    'star': '<path d="m12 2 3 6 7 1-5 5 1 7-6-3-6 3 1-7-5-5 7-1Z"/>',
    'inbox': '<path d="m3 4-2 11v5h22v-5L21 4Z M1 15h6l2 3h6l2-3h6"/>',
    'tag': '<path d="M3 3h8l11 11-8 8L3 11Z"/><circle cx="7.5" cy="7.5" r="1"/>',
    'key': '<circle cx="8" cy="8" r="5"/><path d="m12 12 9 9M16 16l3-3M19 19l3-3"/>',
}


_ICON_CACHE = {}


def icon(name, color=None):
    color = color or theme_colors()['muted']
    key = (name, color)
    if key in _ICON_CACHE:
        return _ICON_CACHE[key]
    svg = ('<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" '
           f'fill="none" stroke="{color}" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">'
           + _PATHS.get(name, _PATHS['key']) + '</svg>')
    pixmap = QPixmap(48, 48)
    pixmap.fill(Qt.transparent)
    painter = QPainter(pixmap)
    QSvgRenderer(QByteArray(svg.encode())).render(painter)
    painter.end()
    pixmap.setDevicePixelRatio(2)
    result = QIcon(pixmap)
    _ICON_CACHE[key] = result
    return result


STYLE_TEMPLATE = '''
QWidget { font-family: "Atkinson Hyperlegible Next", "Noto Sans", sans-serif; font-size: 15px; color: $ink; }
QMainWindow, QDialog, #content, #detailBody { background: $window; }
QFrame, QLabel, QWidget#entryHeader { background: transparent; }
#sidebar { background: $sidebar; border-right: 1px solid $border; }
#brand { font-family: "Spectral"; font-size: 19px; font-weight: 400; }
#brandSubtitle, #muted, #status, #cloud { color: $muted; font-size: 13px; }
#cloud { padding: 8px; }
#nav { background: transparent; border: none; outline: none; }
#nav::item { border: none; }
#listPane { background: $list; border-right: 1px solid $border; }
#paneHeader { border-bottom: 1px solid $divider; }
#search { background: $search; border: none; padding: 8px 10px; }
QPushButton { border: 1px solid transparent; background: $button; padding: 7px 13px; border-radius: 6px; font-weight: 600; min-height: 20px; }
QPushButton:hover { background: $button_hover; }
QPushButton:pressed { background: $button_pressed; }
QPushButton:disabled { color: $disabled; background: $disabled_bg; }
QPushButton:focus { border: 1px solid $accent; }
QPushButton#primary { background: $primary; color: $primary_text; }
QPushButton#primary:hover { background: $primary_hover; }
QPushButton#primary:disabled { background: $primary_disabled; }
QPushButton#danger { color: $danger; background: transparent; }
QPushButton#danger:hover { background: $danger_bg; }
QPushButton#quiet, QPushButton#iconButton { background: transparent; color: $muted; font-weight: 400; }
QPushButton#quiet:hover, QPushButton#iconButton:hover { background: $button; }
QPushButton#iconButton { padding: 6px; min-width: 20px; }
QLineEdit, QTextEdit { border: 1px solid $input_border; border-radius: 6px; background: $surface; padding: 9px; selection-background-color: $selection; selection-color: $ink; }
QLineEdit:focus, QTextEdit:focus { border: 1px solid $accent; }
#entries { background: transparent; border: none; outline: none; }
#entries::item { border: none; }
#entries::item:focus { border: none; }
#details { background: $window; }
#fieldGroup, #metadata, #noteBox { background: $surface; border: 1px solid $surface_border; border-radius: 12px; }
#field { background: transparent; border: none; border-radius: 0; border-bottom: 1px solid $row_border; }
#fieldLast { background: transparent; border: none; }
#fieldLabel { color: $muted; font-size: 13px; }
#secretValue { font-family: "Red Hat Mono", monospace; font-size: 15px; }
QLabel[copyable="true"]:hover { color: $accent_ink; }
QLabel[copyable="true"]:focus { background: $focus_bg; border-radius: 3px; }
#heading { font-family: "Spectral"; font-size: 28px; font-weight: 400; }
#detailTitle { font-family: "Spectral"; font-size: 34px; font-weight: 400; }
#sectionTitle { font-size: 16px; font-weight: 600; padding: 2px; }
#avatar { background: $avatar_bg; color: $accent_ink; border-radius: 30px; font-size: 24px; font-weight: 600; }
#unlockCard { background: $surface; border: 1px solid $surface_border; border-radius: 16px; }
QScrollArea { border: none; background: transparent; }
QSplitter::handle { background: $border; width: 1px; }
QScrollBar:vertical { background: transparent; width: 7px; margin: 0; }
QScrollBar::handle:vertical { background: $scrollbar; border-radius: 3px; min-height: 24px; }
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical { height: 0; }
QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical { background: transparent; }
QCheckBox { spacing: 10px; }
QCheckBox::indicator { width: 20px; height: 20px; border-radius: 10px; border: 1px solid $switch_off; background: $switch_bg; }
QCheckBox::indicator:checked { background: $accent; border: 1px solid $accent; image: none; }
QToolTip { background: $surface; color: $ink; border: 1px solid $border; padding: 6px; }
'''
