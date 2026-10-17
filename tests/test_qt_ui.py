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
        self.directory = tempfile.TemporaryDirectory(dir='/data-ssd/codex-artifacts/safeincloud-linux/work')
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
