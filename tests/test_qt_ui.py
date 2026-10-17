import os
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from PySide6.QtWidgets import QApplication, QDialog, QMessageBox, QLineEdit
from safeincloud_linux.qt_ui import Window, Editor
from safeincloud_linux.vault import Vault, encrypt


class QtDesktopTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.path = Path(self.directory.name) / 'test.db'
        self.path.write_bytes(encrypt(b'<database><label id="9" name="Privat"/><card id="1" title="Mail" star="true"><field name="Login" type="login">demo@example.test</field><field name="Passwort" type="password">sample-only</field><field name="Webseite" type="website">https://example.test</field><label_id>9</label_id></card></database>', 'test'))
        self.window = Window(self.path, settings=False)
        self.window.show()
        self.app.processEvents()

    def tearDown(self):
        if self.window.busy:
            deadline = time.monotonic() + 3
            while self.window.busy and time.monotonic() < deadline:
                self.window.poll()
                time.sleep(.01)
        self.window.close()
        self.window.deleteLater()
        self.app.processEvents()
        self.directory.cleanup()

    def test_unlock_filter_crud_persistence(self):
        self.assertFalse(self.window.unlock('wrong'))
        self.assertTrue(self.window.unlock('test'))
        self.assertEqual(self.window.entries.count(), 1)
        self.window.search.setText('sample-only')
        self.assertEqual(self.window.entries.count(), 0)
        self.window.search.setText('EXAMPLE.TEST')
        self.assertEqual(self.window.entries.count(), 1)
        self.window.search.clear()
        self.window.navigation.setCurrentRow(1)
        self.assertEqual(self.window.entries.count(), 1)
        self.window.navigation.setCurrentRow(2)
        self.assertEqual(self.window.entries.count(), 1)
        self.window.navigation.setCurrentRow(0)
        fields = [dict(name='Login', type='login', value='demo'), dict(name='Passwort', type='password', value='synthetic')]
        self.assertTrue(self.window.apply_entry(None, 'Archive', fields, 'Notiz'))
        ident = self.window.selected_id
        self.assertTrue(self.window.apply_entry(ident, 'Archive edited', fields))
        reopened = Vault.open(self.path, 'test')
        self.assertEqual(len(reopened.cards()), 2)
        reopened.close()
        with patch.object(QMessageBox, 'question', return_value=QMessageBox.Yes):
            self.window.delete_entry()
        self.assertEqual(self.window.entries.count(), 1)
        self.assertTrue(self.window.lock())
        self.assertIsNone(self.window.vault)
        self.assertEqual(self.window.stack.currentIndex(), 0)

    def test_clipboard_and_async_sync(self):
        self.assertTrue(self.window.unlock('test'))
        self.window.copy_value('fake-password')
        self.window.clear_clipboard('fake-password')
        self.assertEqual(self.app.clipboard().text(), '')
        self.window.copy_value('fake-password')
        self.app.clipboard().setText('unrelated')
        self.window.clear_clipboard('fake-password')
        self.assertEqual(self.app.clipboard().text(), 'unrelated')
        calls = []
        self.window.on_sync = lambda vault: calls.append(len(vault.cards())) or 'Abgeglichen'
        self.window.sync()
        self.assertTrue(self.window.busy)
        self.assertFalse(self.window.lock())
        self.assertTrue(self.window.pending_lock)
        self.assertEqual(self.window.stack.currentIndex(), 0)
        self.assertFalse(self.window.unlock('test'))
        deadline = time.monotonic() + 3
        while self.window.busy and time.monotonic() < deadline:
            self.window.poll()
            time.sleep(.01)
        self.assertFalse(self.window.busy)
        self.assertEqual(calls, [1])
        self.assertIsNone(self.window.vault)
        self.assertFalse(self.window.pending_lock)
        self.assertTrue(self.window.unlock('test'))

    def test_editor_preserves_custom_fields(self):
        self.window.unlock('test')
        editor = Editor(self.window, self.window.current_card())
        title, fields, notes = editor.values()
        self.assertEqual(title, 'Mail')
        self.assertEqual(fields[1]['value'], 'sample-only')
        self.assertEqual(editor.inputs[1][1].echoMode(), QLineEdit.Password)
        editor.close()
        editor.deleteLater()

    def test_save_failure_keeps_pending_edits(self):
        self.window.unlock('test')
        with patch.object(self.window.vault, 'save', side_effect=OSError('test failure')):
            self.assertFalse(self.window.apply_entry('1', 'Pending', []))
            self.assertTrue(self.window.vault.dirty)
            self.assertFalse(self.window.lock())
            self.assertIsNotNone(self.window.vault)

    def test_secret_visibility_toggles(self):
        from PySide6.QtWidgets import QLabel, QPushButton
        self.window.unlock('test')
        control = next(b for b in self.window.detail.findChildren(QPushButton) if b.text() == 'Anzeigen')
        control.click()
        self.assertEqual(control.text(), 'Verbergen')
        self.assertTrue(any(w.text() == 'sample-only' for w in self.window.detail.findChildren(QLabel)))
        control.click()
        self.assertEqual(control.text(), 'Anzeigen')
        self.assertFalse(any(w.text() == 'sample-only' and w.isVisible() for w in self.window.detail.findChildren(QLabel)))

    def test_favorite_toggle_persists_without_changing_card_fields(self):
        from PySide6.QtWidgets import QCheckBox
        self.window.unlock('test')
        self.window.navigation.setCurrentRow(1)
        favorite = self.window.detail.findChild(QCheckBox)
        self.assertTrue(favorite.isChecked())
        favorite.click()
        self.assertEqual(self.window.entries.count(), 0)
        reopened = Vault.open(self.path, 'test')
        card = reopened.cards()[0]
        self.assertEqual(card.get('star'), 'false')
        self.assertEqual(card.findtext("field[@type='password']"), 'sample-only')
        self.assertEqual(card.findtext('label_id'), '9')
        reopened.close()

    def test_credential_text_click_copies_underlying_value_without_revealing(self):
        from PySide6.QtCore import Qt
        from PySide6.QtTest import QTest
        from PySide6.QtWidgets import QPushButton
        from safeincloud_linux.qt_ui import CopyValueLabel
        self.window.unlock('test')
        self.app.processEvents()
        values = self.window.detail.findChildren(CopyValueLabel)
        login = next(w for w in values if w.text() == 'demo@example.test')
        password = next(w for w in values if w.objectName() == 'secretValue')
        QTest.mouseClick(login, Qt.LeftButton)
        self.assertEqual(self.app.clipboard().text(), 'demo@example.test')
        QTest.mouseClick(password, Qt.LeftButton)
        self.assertEqual(self.app.clipboard().text(), 'sample-only')
        self.assertEqual(password.text(), '••••••••••••')
        self.assertIn('30 Sekunden', self.window.status.text())
        self.app.clipboard().setText('unrelated')
        QTest.mouseClick(password, Qt.RightButton)
        self.assertEqual(self.app.clipboard().text(), 'unrelated')
        QTest.keyClick(password, Qt.Key_Return)
        self.assertEqual(self.app.clipboard().text(), 'sample-only')
        reveal = next(b for b in self.window.detail.findChildren(QPushButton) if b.text() == 'Anzeigen')
        reveal.click()
        QTest.mouseClick(password, Qt.LeftButton)
        self.assertEqual(self.app.clipboard().text(), 'sample-only')
        self.assertEqual(password.text(), 'sample-only')

    def test_live_system_theme_switch_preserves_unsaved_editor_and_copy(self):
        from PySide6.QtCore import Qt
        from PySide6.QtGui import QPalette
        from PySide6.QtTest import QTest
        from safeincloud_linux.qt_ui import CopyValueLabel
        from safeincloud_linux.qt_system_theme import system_theme
        theme = system_theme()
        original_dark = theme.dark
        self.addCleanup(lambda: theme.set_dark(original_dark))
        self.window.unlock('test')
        encrypted_before = self.path.read_bytes()
        editor = Editor(self.window, self.window.current_card())
        editor.show()
        editor.title_input.setText('Noch nicht gespeichert')
        self.addCleanup(editor.deleteLater)
        password = self.window.detail.findChild(CopyValueLabel, 'secretValue')
        selected = self.window.selected_id
        for scheme in (Qt.ColorScheme.Dark, Qt.ColorScheme.Light):
            # Emit the real Qt notification without changing the user's desktop settings.
            self.app.styleHints().colorSchemeChanged.emit(scheme)
            self.app.processEvents()
            dark = scheme == Qt.ColorScheme.Dark
            self.assertEqual(theme.dark, dark)
            self.assertEqual(self.app.palette().color(QPalette.Window).lightness() < 128, dark)
            self.assertEqual(editor.title_input.text(), 'Noch nicht gespeichert')
            self.assertTrue(editor.isVisible())
            self.assertEqual(self.window.selected_id, selected)
            self.assertEqual(self.path.read_bytes(), encrypted_before)
            QTest.mouseClick(password, Qt.LeftButton)
            self.assertEqual(self.app.clipboard().text(), 'sample-only')
            self.assertEqual(password.text(), '••••••••••••')
        editor.close()

    def test_linux_portal_appearance_notifications(self):
        from PySide6.QtDBus import QDBusVariant
        from safeincloud_linux.qt_system_theme import system_theme
        theme = system_theme()
        original_dark, original_portal = theme.dark, theme._portal_scheme
        def restore():
            theme._portal_scheme = original_portal
            theme.set_dark(original_dark)
        self.addCleanup(restore)
        theme.portal_setting_changed('org.freedesktop.appearance', 'color-scheme', QDBusVariant(QDBusVariant(1)))
        self.assertTrue(theme.dark)
        theme.portal_setting_changed('unrelated.setting', 'color-scheme', QDBusVariant(2))
        self.assertTrue(theme.dark)
        theme.portal_setting_changed('org.freedesktop.appearance', 'color-scheme', QDBusVariant(2))
        self.assertFalse(theme.dark)
