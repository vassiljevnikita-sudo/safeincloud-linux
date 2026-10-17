"""Minimal native Qt desktop for the interoperable password vault."""
import queue
import hmac
import threading
from pathlib import Path

from PySide6.QtCore import Qt, QTimer, QSettings, QEvent, Signal
from PySide6.QtGui import QKeySequence, QShortcut
from PySide6.QtWidgets import (QApplication, QMainWindow, QWidget, QFrame, QHBoxLayout,
    QVBoxLayout, QLabel, QPushButton, QLineEdit, QListWidget, QListWidgetItem,
    QStackedWidget, QScrollArea, QDialog, QFormLayout, QTextEdit,
    QDialogButtonBox, QMessageBox, QSplitter)
from .vault import Vault
from .qt_theme import load_fonts, icon
from .qt_system_theme import system_theme, bind_icon
from .qt_lists import EntryDelegate, NavigationDelegate, FavoriteSwitch, TITLE, LOGIN, FAVORITE, COUNT




def button(text, callback, name=None, icon_name=None):
    value = QPushButton(text)
    if name:
        value.setObjectName(name)
    if icon_name:
        bind_icon(value, icon_name, 'primary_text' if name == 'primary' else 'muted')
    value.setAccessibleName(text)
    value.clicked.connect(callback)
    value.setCursor(Qt.PointingHandCursor)
    return value


def label(text, name=None):
    value = QLabel(text)
    value.setTextFormat(Qt.PlainText)
    if name:
        value.setObjectName(name)
    return value


class CopyValueLabel(QLabel):
    """Copy the underlying credential on click while retaining its masked display."""
    activated = Signal()

    def __init__(self, text, callback, name=None):
        super().__init__(text)
        self.setTextFormat(Qt.PlainText)
        if name:
            self.setObjectName(name)
        self.setProperty('copyable', True)
        self.setCursor(Qt.PointingHandCursor)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setToolTip('Zum Kopieren klicken (oder Eingabetaste drücken)')
        self.activated.connect(callback)

    def mousePressEvent(self, event):
        if event.button() == Qt.LeftButton:
            self.setFocus(Qt.MouseFocusReason)
            event.accept()
        else:
            super().mousePressEvent(event)

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.LeftButton:
            if self.rect().contains(event.position().toPoint()):
                self.activated.emit()
            event.accept()
        else:
            super().mouseReleaseEvent(event)

    def keyPressEvent(self, event):
        if event.key() in (Qt.Key_Return, Qt.Key_Enter, Qt.Key_Space):
            if not event.isAutoRepeat():
                self.activated.emit()
            event.accept()
        else:
            super().keyPressEvent(event)


def public_login(card):
    return next((f.text or '' for f in card.findall('field') if f.get('type', '').lower() in ('login', 'email')), '')


class Editor(QDialog):
    def __init__(self, parent, card=None):
        super().__init__(parent)
        self.setWindowTitle('Neues Passwort' if card is None else 'Eintrag bearbeiten')
        self.resize(500, 520)
        outer = QVBoxLayout(self)
        outer.setContentsMargins(26, 24, 26, 24)
        outer.setSpacing(14)
        outer.addWidget(label(self.windowTitle(), 'heading'))
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        body = QWidget()
        body.setObjectName('detailBody')
        form = QFormLayout(body)
        form.setVerticalSpacing(14)
        self.title_input = QLineEdit(card.get('title', '') if card is not None else '')
        form.addRow('Titel', self.title_input)
        specs = ([dict(name=f.get('name', ''), type=f.get('type', 'text'), value=f.text or '') for f in card.findall('field')]
                 if card is not None else [dict(name='Login', type='login', value=''), dict(name='Passwort', type='password', value=''), dict(name='Webseite', type='website', value='')])
        self.inputs = []
        for spec in specs:
            if '\n' in spec['value']:
                field = QTextEdit()
                field.setPlainText(spec['value'])
                field.setMaximumHeight(90)
            else:
                field = QLineEdit(spec['value'])
                if spec['type'].lower() == 'password':
                    field.setEchoMode(QLineEdit.Password)
            self.inputs.append((spec, field))
            form.addRow(spec['name'], field)
        self.notes = QTextEdit()
        self.notes.setPlainText(card.findtext('notes', '') if card is not None else '')
        self.notes.setMaximumHeight(100)
        form.addRow('Notizen', self.notes)
        scroll.setWidget(body)
        outer.addWidget(scroll)
        buttons = QDialogButtonBox(QDialogButtonBox.Save | QDialogButtonBox.Cancel)
        buttons.button(QDialogButtonBox.Save).setText('Speichern')
        buttons.button(QDialogButtonBox.Save).setObjectName('primary')
        buttons.button(QDialogButtonBox.Cancel).setText('Abbrechen')
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        outer.addWidget(buttons)

    def accept(self):
        if not self.title_input.text().strip():
            self.title_input.setFocus()
            self.title_input.setPlaceholderText('Bitte einen Titel eingeben')
            return
        super().accept()

    def values(self):
        return (self.title_input.text().strip(), [{**spec, 'value': field.toPlainText() if isinstance(field, QTextEdit) else field.text()} for spec, field in self.inputs], self.notes.toPlainText())


class Window(QMainWindow):
    def __init__(self, path, on_sync=None, settings=True):
        super().__init__()
        self.path, self.on_sync = Path(path), on_sync
        self.vault = None
        self.busy = False
        self.pending_lock = False
        self.pending_close = False
        self.selected_id = None
        self.clipboard_value = None
        self.jobs = queue.Queue()
        self.editors = []
        self.settings = QSettings('SafeInCloudLinux', 'Desktop') if settings else None
        load_fonts()
        self.setWindowTitle('SafeInCloud Linux')
        self.resize(1320, 820)
        self.setMinimumSize(920, 620)
        system_theme()
        self.stack = QStackedWidget()
        self.setCentralWidget(self.stack)
        self.build_unlock()
        self.build_main()
        self.idle = QTimer(self)
        self.idle.setSingleShot(True)
        self.idle.setInterval(300000)
        self.idle.timeout.connect(self.auto_lock)
        self.poller = QTimer(self)
        self.poller.timeout.connect(self.poll)
        self.poller.start(75)
        QApplication.instance().installEventFilter(self)
        if self.settings and self.settings.value('geometry'):
            self.restoreGeometry(self.settings.value('geometry'))

    def build_unlock(self):
        page = QWidget()
        page.setObjectName('content')
        layout = QVBoxLayout(page)
        layout.addStretch()
        card = QFrame()
        card.setObjectName('unlockCard')
        card.setFixedWidth(430)
        inner = QVBoxLayout(card)
        inner.setContentsMargins(38, 34, 38, 34)
        inner.setSpacing(18)
        mark = label('')
        bind_icon(mark, 'key', 'accent', 36)
        inner.addWidget(mark)
        inner.addWidget(label('SafeInCloud Linux', 'heading'))
        intro = label('Entsperre deine Datenbank mit deinem\nMasterpasswort.', 'muted')
        inner.addWidget(intro)
        self.password = QLineEdit()
        self.password.setPlaceholderText('Masterpasswort')
        self.password.setEchoMode(QLineEdit.Password)
        self.password.returnPressed.connect(lambda: self.unlock(self.password.text()))
        inner.addWidget(self.password)
        inner.addWidget(button('Datenbank entsperren', lambda: self.unlock(self.password.text()), 'primary'))
        self.unlock_error = label('', 'status')
        self.unlock_error.setWordWrap(True)
        inner.addWidget(self.unlock_error)
        inner.addWidget(label('Dein Masterpasswort bleibt auf diesem Gerät.', 'muted'))
        layout.addWidget(card, alignment=Qt.AlignHCenter)
        layout.addStretch()
        self.stack.addWidget(page)

    def build_main(self):
        page = QWidget()
        outer = QHBoxLayout(page)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        side = QFrame()
        side.setObjectName('sidebar')
        side.setFixedWidth(220)
        nav = QVBoxLayout(side)
        nav.setContentsMargins(0, 0, 0, 12)
        nav.setSpacing(6)
        brand = QWidget()
        brand.setObjectName('paneHeader')
        brand.setFixedHeight(58)
        brand_row = QHBoxLayout(brand)
        brand_row.setContentsMargins(16, 0, 12, 0)
        mark = label('')
        bind_icon(mark, 'key', 'accent', 20)
        brand_row.addWidget(mark)
        brand_row.addWidget(label('SafeInCloud Linux', 'brand'))
        nav.addWidget(brand)
        self.navigation = QListWidget()
        self.navigation.setObjectName('nav')
        self.navigation.setMouseTracking(True)
        self.navigation.setItemDelegate(NavigationDelegate(self.navigation))
        self.navigation.setAccessibleName('Labels und Favoriten')
        self.navigation.currentRowChanged.connect(lambda _: self.refresh_entries())
        nav.addWidget(self.navigation, 1)
        self.cloud = label('', 'cloud')
        self.cloud.setWordWrap(True)
        nav.addWidget(self.cloud)
        outer.addWidget(side)

        split = QSplitter()
        split.setChildrenCollapsible(False)
        split.setHandleWidth(1)
        center = QFrame()
        center.setObjectName('listPane')
        center.setMinimumWidth(270)
        middle = QVBoxLayout(center)
        middle.setContentsMargins(0, 0, 0, 0)
        middle.setSpacing(0)
        search_header = QWidget()
        search_header.setObjectName('paneHeader')
        search_header.setFixedHeight(58)
        search_row = QHBoxLayout(search_header)
        search_row.setContentsMargins(10, 8, 8, 8)
        search_row.setSpacing(6)
        self.search = QLineEdit()
        self.search.setObjectName('search')
        self.search.setPlaceholderText('Suchen')
        self.search.setAccessibleName('Passwörter durchsuchen')
        self.search.setClearButtonEnabled(True)
        self.search_action = self.search.addAction(icon('search'), QLineEdit.LeadingPosition)
        bind_icon(self.search_action, 'search')
        self.search.textChanged.connect(self.refresh_entries)
        search_row.addWidget(self.search, 1)
        self.add_button = button('', lambda: self.edit_entry(True), 'iconButton', 'plus')
        self.add_button.setToolTip('Neues Passwort (Strg+N)')
        self.add_button.setAccessibleName('Neues Passwort')
        search_row.addWidget(self.add_button)
        middle.addWidget(search_header)
        self.entries = QListWidget()
        self.entries.setObjectName('entries')
        self.entries.setAccessibleName('Passworteinträge')
        self.entries.setMouseTracking(True)
        self.entries.setItemDelegate(EntryDelegate(self.entries))
        self.entries.currentItemChanged.connect(self.show_entry)
        middle.addWidget(self.entries, 1)
        self.count = label('', 'muted')
        self.count.setContentsMargins(16, 8, 10, 10)
        middle.addWidget(self.count)
        split.addWidget(center)

        content = QWidget()
        content.setObjectName('content')
        content.setMinimumWidth(390)
        content_layout = QVBoxLayout(content)
        content_layout.setContentsMargins(0, 0, 0, 0)
        content_layout.setSpacing(0)
        top = QWidget()
        top.setObjectName('paneHeader')
        top.setFixedHeight(58)
        top_row = QHBoxLayout(top)
        top_row.setContentsMargins(16, 6, 16, 6)
        top_row.addStretch()
        self.sync_button = button('Drive abgleichen', self.sync, 'quiet', 'sync')
        self.sync_button.setVisible(self.on_sync is not None)
        top_row.addWidget(self.sync_button)
        top_row.addWidget(button('Sperren', self.lock, 'quiet', 'lock'))
        content_layout.addWidget(top)
        detail_scroll = QScrollArea()
        detail_scroll.setWidgetResizable(True)
        detail_scroll.setAlignment(Qt.AlignHCenter | Qt.AlignTop)
        self.detail = QFrame()
        self.detail.setObjectName('details')
        self.detail.setMaximumWidth(676)
        self.detail_layout = QVBoxLayout(self.detail)
        self.detail_layout.setContentsMargins(28, 30, 28, 22)
        self.detail_layout.setSpacing(18)
        detail_scroll.setWidget(self.detail)
        content_layout.addWidget(detail_scroll, 1)
        self.status = label('Lokal gespeichert', 'status')
        self.status.setWordWrap(True)
        self.status.setContentsMargins(20, 8, 20, 12)
        self.status.setAccessibleName('Speicher- und Synchronisationsstatus')
        content_layout.addWidget(self.status)
        split.addWidget(content)
        split.setStretchFactor(0, 0)
        split.setStretchFactor(1, 1)
        split.setSizes([330, 770])
        outer.addWidget(split, 1)
        self.stack.addWidget(page)
        self.empty_detail()
        for key, callback in [('Ctrl+F', self.search.setFocus),
                              ('Ctrl+N', lambda: self.edit_entry(True)),
                              ('Ctrl+E', self.edit_entry), ('Ctrl+L', self.lock)]:
            shortcut = QShortcut(QKeySequence(key), self)
            shortcut.activated.connect(callback)

    def eventFilter(self, obj, event):
        if self.vault and event.type() in (QEvent.KeyPress, QEvent.MouseButtonPress, QEvent.Wheel):
            self.idle.start()
        return super().eventFilter(obj, event)

    def unlock(self, password):
        if self.busy:
            return False
        self.password.clear()
        if self.vault:
            if hmac.compare_digest(password.encode(), self.vault.password.encode()):
                self.stack.setCurrentIndex(1)
                self.idle.start()
                self.unlock_error.clear()
                return True
            self.unlock_error.setText('Das Masterpasswort ist falsch.')
            return False
        try:
            self.vault = Vault.open(self.path, password)
        except Exception:
            self.unlock_error.setText('Das Passwort ist falsch oder die Datei wird noch nicht unterstützt.')
            return False
        self.unlock_error.clear()
        self.stack.setCurrentIndex(1)
        self.idle.start()
        self.refresh_navigation()
        self.refresh_entries()
        self.cloud.setText('●  Google Drive\nVerbunden · verschlüsselte Datei' if self.on_sync else '●  Lokale Datenbank\nAuf diesem Gerät gespeichert')
        self.status.setText('Entsperrt · Änderungen werden automatisch lokal gespeichert')
        if getattr(self.on_sync, 'auto_sync_on_unlock', False) is True:
            QTimer.singleShot(0, self.sync)
        return True

    def refresh_navigation(self):
        previous = self.navigation.currentRow()
        self.navigation.blockSignals(True)
        self.navigation.clear()
        cards = self.vault.cards()
        all_entries = QListWidgetItem('Alle Einträge')
        all_entries.setData(COUNT, len(cards))
        self.navigation.addItem(all_entries)
        favorites = QListWidgetItem('Favoriten')
        favorites.setData(COUNT, sum(c.get('star', '').lower() == 'true' for c in cards))
        self.navigation.addItem(favorites)
        for item in self.vault.root.findall('label'):
            entry = QListWidgetItem(item.get('name', 'Label'))
            entry.setData(Qt.UserRole, item.get('id'))
            entry.setData(COUNT, sum(item.get('id') in [v.text for v in c.findall('label_id')] for c in cards))
            self.navigation.addItem(entry)
        self.navigation.setCurrentRow(max(0, min(previous, self.navigation.count() - 1)))
        self.navigation.blockSignals(False)

    def refresh_entries(self, *_):
        if not self.vault:
            return
        selected = self.selected_id
        query = self.search.text().casefold()
        row = self.navigation.currentRow()
        nav_item = self.navigation.currentItem()
        label_id = nav_item.data(Qt.UserRole) if nav_item else None
        cards = []
        for card in self.vault.cards():
            if row == 1 and card.get('star', '').lower() != 'true':
                continue
            if row > 1 and label_id not in [e.text for e in card.findall('label_id')]:
                continue
            text = ' '.join([card.get('title', '')] + [f.text or '' for f in card.findall('field') if f.get('type', '').lower() in ('login', 'email', 'website', 'url')])
            if query in text.casefold():
                cards.append(card)
        cards.sort(key=lambda c: c.get('title', '').casefold())
        self.entries.blockSignals(True)
        self.entries.clear()
        selection = 0
        for i, card in enumerate(cards):
            title = card.get('title', 'Ohne Titel')
            entry = QListWidgetItem(f'{title[:1].upper()}    {title}\n      {public_login(card) or "Passwort-Eintrag"}')
            entry.setData(Qt.UserRole, card.get('id'))
            entry.setData(TITLE, title)
            entry.setData(LOGIN, public_login(card))
            entry.setData(FAVORITE, card.get('star', '').lower() == 'true')
            self.entries.addItem(entry)
            if card.get('id') == selected:
                selection = i
        self.entries.setCurrentRow(selection)
        self.entries.blockSignals(False)
        self.count.setText(f'{len(cards)} Eintrag' if len(cards) == 1 else f'{len(cards)} Einträge')
        self.show_entry(self.entries.currentItem())

    def current_card(self):
        return next((c for c in self.vault.cards() if c.get('id') == self.selected_id), None) if self.vault else None

    def clear_detail(self):
        while self.detail_layout.count():
            item = self.detail_layout.takeAt(0)
            if item.widget():
                item.widget().hide()
                item.widget().deleteLater()

    def empty_detail(self):
        self.clear_detail()
        self.detail_layout.addStretch()
        title = label('Deine Passwörter, übersichtlich.', 'heading')
        title.setWordWrap(True)
        self.detail_layout.addWidget(title)
        hint = label('Wähle links einen Eintrag aus oder\nfüge dein erstes Passwort hinzu.', 'muted')
        self.detail_layout.addWidget(hint)
        self.detail_layout.addStretch()

    def show_entry(self, item=None, *_):
        self.selected_id = item.data(Qt.UserRole) if item else None
        card = self.current_card()
        if card is None:
            self.empty_detail()
            return
        self.clear_detail()
        header = QWidget()
        header.setObjectName('entryHeader')
        head = QHBoxLayout(header)
        head.setContentsMargins(0, 0, 0, 0)
        head.setSpacing(18)
        avatar = label(card.get('title', '?')[:1].upper(), 'avatar')
        avatar.setFixedSize(60, 60)
        avatar.setAlignment(Qt.AlignCenter)
        head.addWidget(avatar)
        title = label(card.get('title', ''), 'detailTitle')
        title.setWordWrap(True)
        head.addWidget(title, 1)
        self.edit_button = button('Bearbeiten', self.edit_entry, None, 'edit')
        self.edit_button.setEnabled(not self.busy)
        head.addWidget(self.edit_button)
        self.detail_layout.addWidget(header)
        self.detail_layout.addSpacing(6)
        self.detail_layout.addWidget(label('Zugangsdaten', 'sectionTitle'))
        group = QFrame()
        group.setObjectName('fieldGroup')
        rows = QVBoxLayout(group)
        rows.setContentsMargins(0, 0, 0, 0)
        rows.setSpacing(0)
        fields = card.findall('field')
        for index, field in enumerate(fields):
            value = field.text or ''
            box = QFrame()
            box.setObjectName('fieldLast' if index == len(fields) - 1 else 'field')
            line = QHBoxLayout(box)
            line.setContentsMargins(15, 11, 9, 11)
            line.setSpacing(5)
            values = QVBoxLayout()
            values.setSpacing(3)
            name = field.get('name', '')
            if field.get('type', '').lower() in ('login', 'email'):
                name = 'Benutzername / E-Mail'
            values.addWidget(label(name, 'fieldLabel'))
            public = field.get('type', '').lower() in ('login', 'email', 'website', 'url')
            text = CopyValueLabel(value if public else '••••••••••••',
                                  lambda v=value: self.copy_value(v),
                                  None if public else 'secretValue')
            text.setAccessibleName(name + ' kopieren')
            text.setWordWrap(True)
            values.addWidget(text)
            line.addLayout(values, 1)
            if not public:
                eye = button('Anzeigen', lambda: None, 'quiet', 'eye')
                eye.setAccessibleName(name + ' anzeigen')
                eye.clicked.connect(lambda checked=False, w=text, v=value, b=eye: self.toggle_secret(w, v, b))
                line.addWidget(eye)
            copy = button('', lambda checked=False, v=value: self.copy_value(v), 'iconButton', 'copy')
            copy.setToolTip(name + ' kopieren')
            copy.setAccessibleName(name + ' kopieren')
            line.addWidget(copy)
            rows.addWidget(box)
        self.detail_layout.addWidget(group)

        metadata = QFrame()
        metadata.setObjectName('metadata')
        meta = QVBoxLayout(metadata)
        meta.setContentsMargins(15, 12, 15, 12)
        meta.setSpacing(16)
        labels = {e.get('id'): e.get('name', '') for e in self.vault.root.findall('label')}
        names = ', '.join(labels.get(v.text, '') for v in card.findall('label_id') if v.text in labels)
        label_row = QHBoxLayout()
        label_row.addWidget(label('Label'))
        label_names = label(names or 'Ohne Label', 'muted')
        label_names.setWordWrap(True)
        label_row.addWidget(label_names, 1, Qt.AlignRight)
        meta.addLayout(label_row)
        favorite_row = QHBoxLayout()
        favorite_row.addWidget(label('Favorit'))
        favorite_row.addStretch()
        favorite = FavoriteSwitch()
        favorite.setAccessibleName('Als Favorit markieren')
        favorite.setChecked(card.get('star', '').lower() == 'true')
        favorite.setEnabled(not self.busy)
        favorite.clicked.connect(self.toggle_favorite)
        favorite_row.addWidget(favorite)
        meta.addLayout(favorite_row)
        self.detail_layout.addWidget(metadata)
        notes = card.findtext('notes', '')
        if notes:
            self.detail_layout.addWidget(label('Notiz', 'sectionTitle'))
            note_box = QFrame()
            note_box.setObjectName('noteBox')
            note_layout = QVBoxLayout(note_box)
            note_layout.setContentsMargins(15, 13, 15, 13)
            note = label(notes)
            note.setWordWrap(True)
            note.setTextInteractionFlags(Qt.TextSelectableByMouse | Qt.TextSelectableByKeyboard)
            note_layout.addWidget(note)
            self.detail_layout.addWidget(note_box)
        self.detail_layout.addStretch(1)
        footer = QWidget()
        controls = QHBoxLayout(footer)
        controls.setContentsMargins(0, 8, 0, 0)
        controls.addStretch()
        self.delete_button = button('Eintrag löschen', self.delete_entry, 'danger')
        self.delete_button.setEnabled(not self.busy)
        controls.addWidget(self.delete_button)
        self.detail_layout.addWidget(footer)

    def toggle_favorite(self, checked):
        card = self.current_card()
        if card is None or self.busy:
            return
        fields = [dict(name=f.get('name', ''), type=f.get('type', 'text'), value=f.text or '') for f in card.findall('field')]
        self.vault.upsert(card.get('id'), card.get('title', ''), fields, card.findtext('notes', ''))
        card.set('star', 'true' if checked else 'false')
        self.persist()

    def toggle_secret(self, widget, value, control):
        reveal = control.text() == 'Anzeigen'
        widget.setText(value if reveal else '••••••••••••')
        control.setText('Verbergen' if reveal else 'Anzeigen')
        bind_icon(control, 'hide' if reveal else 'eye')
        control.setAccessibleName('Passwort verbergen' if reveal else 'Passwort anzeigen')

    def copy_value(self, value):
        QApplication.clipboard().setText(value)
        self.clipboard_value = value
        QTimer.singleShot(30000, lambda v=value: self.clear_clipboard(v))
        self.status.setText('Kopiert · Zwischenablage wird nach 30 Sekunden geleert')

    def clear_clipboard(self, value):
        clipboard = QApplication.clipboard()
        if clipboard.text() == value:
            clipboard.clear()
        if self.clipboard_value == value:
            self.clipboard_value = None

    def edit_entry(self, new=False):
        if not self.vault or self.busy:
            return
        card = None if new else self.current_card()
        if card is None and not new:
            return
        dialog = Editor(self, card)
        self.editors.append(dialog)
        if dialog.exec() == QDialog.Accepted and self.vault:
            title, fields, notes = dialog.values()
            self.apply_entry(None if card is None else card.get('id'), title, fields, notes)
        self.editors.remove(dialog)
        dialog.deleteLater()

    def apply_entry(self, ident, title, fields, notes=''):
        if not self.vault or self.busy:
            return False
        try:
            self.selected_id = self.vault.upsert(ident, title, fields, notes)
            return self.persist()
        except Exception as exc:
            self.status.setText('Änderung nicht gespeichert: ' + str(exc))
            return False

    def persist(self):
        try:
            self.vault.save()
        except Exception as exc:
            self.status.setText('Speichern fehlgeschlagen. Änderungen bleiben im entsperrten Tresor: ' + str(exc))
            self.refresh_entries()
            return False
        self.refresh_navigation()
        self.refresh_entries()
        self.status.setText('Lokal gespeichert · verschlüsselte Sicherung erstellt')
        if self.on_sync:
            self.sync()
        return True

    def delete_entry(self):
        if not self.vault or self.busy or self.current_card() is None:
            return
        if QMessageBox.question(self, 'Eintrag löschen', 'Diesen Eintrag in den Papierkorb verschieben?', QMessageBox.Yes | QMessageBox.No) == QMessageBox.Yes:
            self.vault.delete(self.selected_id)
            self.persist()

    def sync(self):
        if self.busy or not self.vault or not self.on_sync:
            return
        self.busy = True
        self.sync_button.setEnabled(False)
        self.add_button.setEnabled(False)
        self.navigation.setEnabled(False)
        self.search.setEnabled(False)
        self.entries.setEnabled(False)
        self.show_entry(self.entries.currentItem())
        self.status.setText('Google Drive wird abgeglichen …')
        target = self.vault
        def work():
            try:
                self.jobs.put((True, self.on_sync(target) or 'Mit Google Drive abgeglichen'))
            except Exception as exc:
                self.jobs.put((False, str(exc)))
        threading.Thread(target=work, daemon=True).start()

    def poll(self):
        try:
            success, text = self.jobs.get_nowait()
        except queue.Empty:
            return
        self.busy = False
        if self.pending_lock:
            self.pending_lock = False
            self.lock()
            if self.pending_close:
                self.pending_close = False
                self.close()
            return
        self.sync_button.setEnabled(True)
        self.add_button.setEnabled(True)
        self.navigation.setEnabled(True)
        self.search.setEnabled(True)
        self.entries.setEnabled(True)
        if self.vault:
            self.refresh_navigation()
            self.refresh_entries()
        self.status.setText(text if success else 'Lokal gespeichert · Drive-Abgleich ausstehend: ' + text)

    def auto_lock(self):
        self.lock()

    def lock(self):
        if self.busy:
            self.pending_lock = True
            self.idle.stop()
            self.stack.setCurrentIndex(0)
            self.password.clear()
            self.unlock_error.setText('Der Drive-Abgleich wird abgeschlossen. Dein Tresor wird gesperrt.')
            if self.clipboard_value is not None:
                self.clear_clipboard(self.clipboard_value)
            return False
        if self.vault and self.vault.dirty:
            try:
                self.vault.save()
            except Exception as exc:
                self.status.setText('Sperren ausgesetzt: Änderungen konnten nicht gespeichert werden. ' + str(exc))
                self.unlock_error.setText('Änderungen konnten nicht gespeichert werden. Zum Wiederherstellen bitte erneut entsperren.')
                self.idle.start()
                return False
        for dialog in self.editors:
            dialog.reject()
        self.idle.stop()
        if self.vault:
            self.vault.close()
        self.vault = None
        self.selected_id = None
        self.entries.clear()
        self.search.clear()
        self.empty_detail()
        if self.clipboard_value is not None:
            self.clear_clipboard(self.clipboard_value)
        self.sync_button.setEnabled(True)
        self.add_button.setEnabled(True)
        self.navigation.setEnabled(True)
        self.search.setEnabled(True)
        self.entries.setEnabled(True)
        self.unlock_error.clear()
        self.stack.setCurrentIndex(0)
        self.password.setFocus()
        return True

    def closeEvent(self, event):
        if not self.lock():
            if self.busy:
                self.pending_close = True
            event.ignore()
            return
        if self.settings:
            self.settings.setValue('geometry', self.saveGeometry())
        QApplication.instance().removeEventFilter(self)
        event.accept()


def run(path, on_sync=None):
    app = QApplication.instance() or QApplication([])
    app.setQuitOnLastWindowClosed(True)
    window = Window(path, on_sync)
    window.show()
    return app.exec()
