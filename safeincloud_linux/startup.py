"""Automatic Safe app-folder discovery before local vault unlocking."""
import os
from pathlib import Path
import threading
from PySide6.QtCore import QObject, Signal, QTimer
from PySide6.QtGui import QIcon
from PySide6.QtWidgets import QApplication, QDialog, QVBoxLayout, QHBoxLayout, QLabel, QListWidget, QFileDialog
from .official_drive import OfficialDrive
from .qt_ui import button, label, run
from .qt_system_theme import system_theme
from .qt_theme import load_fonts


class Delivery(QObject):
    done = Signal(object, object)


class SourceWindow(QDialog):
    def __init__(self, directory, drive=None):
        super().__init__()
        self.directory = directory
        self.drive = drive or OfficialDrive()
        self.source = None
        self.candidates = []
        self.callback = None
        load_fonts()
        self.setWindowTitle('SafeInCloud Linux')
        self.setMinimumSize(640, 440)
        system_theme()
        layout = QVBoxLayout(self)
        layout.setContentsMargins(32, 28, 32, 24)
        layout.setSpacing(14)
        layout.addWidget(label('SafeInCloud Linux', 'brand'))
        layout.addWidget(label('Deine Passwörter.\nAuf deinen Geräten.', 'heading'))
        self.status = label('Verbinde Google Drive. Deine Safe-Datenbank wird danach automatisch geöffnet.', 'status')
        self.status.setWordWrap(True)
        layout.addWidget(self.status)
        actions = QHBoxLayout()
        self.connect_button = button('Google Drive verbinden', self.connect, 'primary')
        self.search_button = button('Erneut suchen', self.search)
        self.local_button = button('Lokale Datei', self.local, 'quiet')
        self.buttons = [self.connect_button, self.search_button, self.local_button]
        for item in self.buttons:
            actions.addWidget(item)
        layout.addLayout(actions)
        self.listing = QListWidget()
        self.listing.setObjectName('entries')
        self.listing.hide()
        layout.addWidget(self.listing, 1)
        self.open_button = button('Ausgewählte Datenbank öffnen', self.open_selected, 'primary')
        self.open_button.hide()
        self.buttons.append(self.open_button)
        layout.addWidget(self.open_button)
        layout.addStretch()
        notice = label('Unabhängiger Linux-Client. Die Anmeldung verwendet deine lokal vorhandene\n'
                       'Safe-App-Konfiguration; Google zeigt dabei SafeInCloud. Dein Masterpasswort bleibt lokal.', 'muted')
        notice.setWordWrap(True)
        layout.addWidget(notice)
        self.delivery = Delivery(self)
        self.delivery.done.connect(self.completed)
        if self.drive.configured():
            QTimer.singleShot(100, self.search)

    def job(self, work, callback, message):
        self.callback = callback
        self.status.setText(message)
        for item in self.buttons:
            item.setEnabled(False)
        def worker():
            try:
                result, error = work(), None
            except Exception as exc:
                result, error = None, str(exc)
            try:
                self.delivery.done.emit(result, error)
            except RuntimeError:
                pass  # Window closed during login/download.
        threading.Thread(target=worker, daemon=True).start()

    def completed(self, result, error):
        for item in self.buttons:
            item.setEnabled(True)
        callback, self.callback = self.callback, None
        if error:
            self.status.setText(error)
        elif callback:
            callback(result)

    def connect(self):
        self.job(self.drive.connect, lambda _: self.search(), 'Bitte die Google-Anmeldung im Browser abschließen …')

    def search(self):
        self.job(self.drive.discover, self.discovered, 'Deine Safe-Datenbank wird gesucht …')

    def discovered(self, rows):
        self.candidates = rows
        self.listing.clear()
        for row in rows:
            self.listing.addItem(row['path'])
        preferred = self.drive.preferred(rows)
        if preferred:
            self.open(preferred)
        elif rows:
            self.status.setText('Mehrere Datenbanken gefunden. Wähle einmal deine Hauptdatenbank aus.')
            self.listing.show()
            self.open_button.show()
        else:
            self.status.setText('Keine Safe-Datenbank im privaten App-Ordner gefunden. Verwende dasselbe Google-Konto wie in Safe.')

    def open_selected(self):
        row = self.listing.currentRow()
        if row >= 0:
            self.open(self.candidates[row])

    def open(self, candidate):
        self.job(lambda: self.drive.open_candidate(candidate, self.directory), self.downloaded,
                 'Deine verschlüsselte Safe-Datenbank wird geladen …')

    def downloaded(self, session):
        self.source = (str(session.path), session)
        self.accept()

    def local(self):
        path, _ = QFileDialog.getOpenFileName(self, 'Safe-Datenbank öffnen', '', 'SafeInCloud (*.db);;Alle Dateien (*)')
        if path:
            self.source = (path, None)
            self.accept()


def choose_source(directory, on_source=None):
    app = QApplication.instance() or QApplication([])
    app.setQuitOnLastWindowClosed(False)
    window = SourceWindow(directory)
    window.exec()
    app.setQuitOnLastWindowClosed(True)
    if window.source:
        path, session = window.source
        if on_source:
            on_source(session)
        return path
    return None


def launch(path=None):
    app = QApplication.instance() or QApplication([])
    load_fonts()
    app.setApplicationName('SafeInCloud Linux')
    app.setDesktopFileName('safeincloud-linux')
    app.setWindowIcon(QIcon(str(Path(__file__).parent / 'assets/icon.svg')))
    app.setOrganizationName('SafeInCloud Linux')
    if path:
        return run(path)
    directory = Path(os.environ.get('XDG_DATA_HOME', Path.home() / '.local/share')) / 'safeincloud-linux/vaults'
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    directory.chmod(0o700)
    drive = OfficialDrive()
    cached = drive.cached_session(directory) if drive.configured() else None
    if cached is not None:
        return run(cached.path, on_sync=cached)
    sessions = []
    selected = choose_source(directory, sessions.append)
    if selected:
        return run(selected, on_sync=sessions[0] if sessions else None)
    return 0
