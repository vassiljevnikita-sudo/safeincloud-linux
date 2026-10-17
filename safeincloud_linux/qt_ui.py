"""Minimal native Qt desktop for the interoperable password vault."""
import queue
import hmac
import threading
from pathlib import Path

from PySide6.QtCore import Qt, QTimer, QSettings, QEvent
from PySide6.QtGui import QColor
from PySide6.QtWidgets import (QApplication, QMainWindow, QWidget, QFrame, QHBoxLayout,
    QVBoxLayout, QLabel, QPushButton, QLineEdit, QListWidget, QListWidgetItem,
    QStackedWidget, QScrollArea, QDialog, QFormLayout, QTextEdit,
    QDialogButtonBox, QMessageBox, QSplitter)
from .vault import Vault

STYLE = '''
QWidget { font-family: "Inter", "Noto Sans", sans-serif; font-size: 14px; color: #23334c; }
QMainWindow, #content { background: #f4f6fa; }
#sidebar { background: #14243c; } #sidebar QLabel { color: #9faec2; }
#sidebar QLabel#brand { color: white; font-size: 19px; font-weight: 700; } #brandSubtitle { color: #8da3c0; font-size: 12px; }
#sidebar QListWidget { background: transparent; border: none; color: #bdcadc; outline: none; }
#sidebar QListWidget::item { padding: 14px 12px; border-radius: 8px; margin: 2px 0; }
#sidebar QListWidget::item:selected { background: #294263; color: white; }
#sidebar #cloud { background: #20344f; border-radius: 10px; padding: 14px; color: #c0d0e6; font-size: 12px; }
QPushButton { border: 1px solid #dce3ed; background: white; padding: 9px 14px; border-radius: 8px; font-weight: 600; }
QPushButton:hover { background: #edf3ff; border-color: #adc3e8; }
QPushButton:disabled { color: #9fa9b8; background: #f1f3f7; }
QPushButton#primary { background: #2563eb; border-color: #2563eb; color: white; }
QPushButton#primary:hover { background: #1d4ed8; }
QPushButton#danger { color: #bb4455; border: none; background: transparent; }
QPushButton#quiet { border: none; background: transparent; color: #667994; }
QLineEdit, QTextEdit { border: 1px solid #dfe5ee; border-radius: 8px; background: white; padding: 10px; selection-background-color: #bfd5ff; }
QLineEdit:focus, QTextEdit:focus { border: 1px solid #6595ee; }
#entries { background: transparent; border: none; outline: none; }
#entries::item { background: white; padding: 16px 12px; margin: 3px 0; border: 1px solid #e5eaf1; border-radius: 10px; }
#entries::item:selected { background: #eaf1ff; border: 1px solid #8eaff0; color: #1646a1; }
#details { background: white; border-radius: 14px; }
#field { background: #f7f9fc; border: 1px solid #e9eef5; border-radius: 10px; }
#field QLabel { border: none; background: transparent; }
#muted { color: #7b8ba2; font-size: 12px; }
#heading { font-size: 27px; font-weight: 700; color: #1d2c45; }
#detailTitle { font-size: 28px; font-weight: 700; }
#avatar { background: #eaf0ff; color: #3864c0; border-radius: 14px; font-size: 26px; font-weight: 700; }
#unlockCard { background: white; border: 1px solid #e2e8f2; border-radius: 18px; }
#status { color: #74849a; font-size: 12px; }
QScrollArea { border: none; background: transparent; }
QSplitter::handle { background: transparent; width: 14px; }
QDialog { background: #f8fafd; }
#detailBody { background: white; }
QScrollBar:vertical { background: transparent; width: 6px; margin: 0; }
QScrollBar::handle:vertical { background: #d7dfe9; border-radius: 3px; min-height: 24px; }
QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical { height: 0; }
QScrollBar::add-page:vertical, QScrollBar::sub-page:vertical { background: transparent; }
'''


def button(text, callback, name=None):
    value = QPushButton(text)
    if name:
        value.setObjectName(name)
    value.clicked.connect(callback)
    value.setCursor(Qt.PointingHandCursor)
    return value


def label(text, name=None):
    value = QLabel(text)
    if name:
        value.setObjectName(name)
    return value


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
        self.setWindowTitle('SafeInCloud Linux')
        self.resize(1180, 760)
        self.setMinimumSize(920, 620)
        self.setStyleSheet(STYLE)
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
        inner.addWidget(label('▣', 'heading'))
        inner.addWidget(label('Deine Passwörter.\nAn einem sicheren Ort.', 'heading'))
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
        inner.addWidget(label('Verschlüsselt gespeichert · Lokal entsperrt', 'muted'))
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
        nav.setContentsMargins(22, 30, 22, 24)
        nav.setSpacing(14)
        nav.addWidget(label('▣  SafeInCloud', 'brand'))
        nav.addWidget(label('DEIN PASSWORT-TRESOR', 'brandSubtitle'))
        nav.addSpacing(26)
        nav.addWidget(label('BIBLIOTHEK', 'brandSubtitle'))
        self.navigation = QListWidget()
        self.navigation.currentRowChanged.connect(lambda _: self.refresh_entries())
        nav.addWidget(self.navigation)
        self.cloud = label('', 'cloud')
        self.cloud.setWordWrap(True)
        nav.addWidget(self.cloud)
        nav.addWidget(label('Linux · Lokal verschlüsselt', 'brandSubtitle'))
        outer.addWidget(side)
        content = QWidget()
        content.setObjectName('content')
        content_layout = QVBoxLayout(content)
        content_layout.setContentsMargins(28, 28, 28, 20)
        content_layout.setSpacing(22)
        top = QHBoxLayout()
        heading = QVBoxLayout()
        heading.setSpacing(4)
        heading.addWidget(label('Passwörter', 'heading'))
        heading.addWidget(label('Alles Wichtige. Einfach griffbereit.', 'muted'))
        top.addLayout(heading)
        top.addStretch()
        self.sync_button = button('↻  Drive abgleichen', self.sync)
        self.sync_button.setVisible(self.on_sync is not None)
        top.addWidget(self.sync_button)
        top.addWidget(button('Sperren', self.lock, 'quiet'))
        self.add_button = button('+  Neues Passwort', lambda: self.edit_entry(True), 'primary')
        top.addWidget(self.add_button)
        content_layout.addLayout(top)
        split = QSplitter()
        split.setHandleWidth(18)
        center = QWidget()
        middle = QVBoxLayout(center)
        middle.setContentsMargins(0, 0, 0, 0)
        middle.setSpacing(12)
        self.search = QLineEdit()
        self.search.setPlaceholderText('Passwörter durchsuchen …')
        self.search.textChanged.connect(self.refresh_entries)
        middle.addWidget(self.search)
        self.count = label('', 'muted')
        middle.addWidget(self.count)
        self.entries = QListWidget()
        self.entries.setObjectName('entries')
        self.entries.setSpacing(3)
        self.entries.currentItemChanged.connect(self.show_entry)
        middle.addWidget(self.entries)
        split.addWidget(center)
        self.detail = QFrame()
        self.detail.setObjectName('details')
        self.detail_layout = QVBoxLayout(self.detail)
        self.detail_layout.setContentsMargins(27, 28, 27, 24)
        split.addWidget(self.detail)
        split.setSizes([340, 480])
        content_layout.addWidget(split, 1)
        self.status = label('Lokal gespeichert', 'status')
        content_layout.addWidget(self.status)
        outer.addWidget(content, 1)
        self.stack.addWidget(page)
        self.empty_detail()

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
        self.navigation.addItem('▤   Alle Einträge')
        self.navigation.addItem('☆   Favoriten')
        for item in self.vault.root.findall('label'):
            entry = QListWidgetItem('○   ' + item.get('name', 'Label'))
            entry.setData(Qt.UserRole, item.get('id'))
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
            self.entries.addItem(entry)
            if card.get('id') == selected:
                selection = i
        self.entries.setCurrentRow(selection)
        self.entries.blockSignals(False)
        self.count.setText(f'{len(cards)} EINTRÄGE')
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
        avatar = label(card.get('title', '?')[:1].upper(), 'avatar')
        avatar.setFixedSize(62, 62)
        avatar.setAlignment(Qt.AlignCenter)
        self.detail_layout.addWidget(avatar)
        title = label(card.get('title', ''), 'detailTitle')
        title.setWordWrap(True)
        self.detail_layout.addWidget(title)
        self.detail_layout.addWidget(label('ZUGANGSDATEN', 'muted'))
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        body = QWidget()
        body.setObjectName('detailBody')
        scroll.viewport().setStyleSheet('background: white;')
        rows = QVBoxLayout(body)
        rows.setContentsMargins(0, 10, 0, 0)
        rows.setSpacing(12)
        for field in card.findall('field'):
            value = field.text or ''
            box = QFrame()
            box.setObjectName('field')
            layout = QVBoxLayout(box)
            layout.setContentsMargins(15, 12, 12, 12)
            layout.addWidget(label(field.get('name', ''), 'muted'))
            line = QHBoxLayout()
            public = field.get('type', '').lower() in ('login', 'email', 'website', 'url')
            text = label(value if public else '••••••••••••')
            text.setWordWrap(True)
            line.addWidget(text, 1)
            if not public:
                eye = button('Anzeigen', lambda: None, 'quiet')
                eye.clicked.connect(lambda checked=False, w=text, v=value, b=eye: self.toggle_secret(w, v, b))
                line.addWidget(eye)
            line.addWidget(button('Kopieren', lambda checked=False, v=value: self.copy_value(v), 'quiet'))
            layout.addLayout(line)
            rows.addWidget(box)
        notes = card.findtext('notes', '')
        if notes:
            rows.addWidget(label('NOTIZEN', 'muted'))
            note = label(notes)
            note.setWordWrap(True)
            rows.addWidget(note)
        rows.addStretch()
        scroll.setWidget(body)
        self.detail_layout.addWidget(scroll, 1)
        footer = QWidget()
        controls = QHBoxLayout(footer)
        controls.setContentsMargins(0, 8, 0, 0)
        self.edit_button = button('Bearbeiten', self.edit_entry)
        self.delete_button = button('Eintrag löschen', self.delete_entry, 'danger')
        self.edit_button.setEnabled(not self.busy)
        self.delete_button.setEnabled(not self.busy)
        controls.addWidget(self.edit_button)
        controls.addStretch()
        controls.addWidget(self.delete_button)
        self.detail_layout.addWidget(footer)

    def toggle_secret(self, widget, value, control):
        reveal = control.text() == 'Anzeigen'
        widget.setText(value if reveal else '••••••••••••')
        control.setText('Verbergen' if reveal else 'Anzeigen')

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
