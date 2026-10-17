"""Credential access stays revoked even when persistence cannot finish."""
import os
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
import tempfile
import unittest
import io
from contextlib import redirect_stdout, redirect_stderr
from pathlib import Path
from unittest.mock import patch

from PySide6.QtCore import Qt, QEvent
from PySide6.QtGui import QKeyEvent
from PySide6.QtWidgets import QApplication, QDialog, QLabel, QMessageBox, QPushButton
from safeincloud_linux.qt_ui import Window, Editor, CopyValueLabel
from safeincloud_linux.vault import encrypt, Vault


class LockBoundaryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = Path(self.directory.name) / 'synthetic.db'
        self.path.write_bytes(encrypt(b'<database><card id="1" title="Synthetic">'
            b'<field name="Password" type="password">synthetic-only</field>'
            b'<notes>private demo note</notes></card></database>', 'test-only'))
        self.window = Window(self.path, settings=False)
        self.window.show()
        self.app.processEvents()

    def tearDown(self):
        self.window.busy = False
        self.window.on_sync = None
        self.window.close()
        self.window.deleteLater()
        self.app.processEvents()

    def pending_edits(self):
        self.assertTrue(self.window.unlock('test-only'))
        self.window.vault.upsert('1', 'Pending', [dict(name='Password', type='password', value='unsaved-demo')])

    def assert_access_blocked(self):
        window = self.window
        self.assertTrue(window.locked)
        self.assertEqual(window.stack.currentIndex(), 0)
        self.assertEqual(window.entries.count(), 0)
        self.assertEqual(window.navigation.count(), 0)
        self.assertIsNone(window.current_card())
        self.assertFalse(window.idle.isActive())
        self.app.clipboard().setText('unrelated')
        window.copy_value('synthetic-only')
        self.assertEqual(self.app.clipboard().text(), 'unrelated')
        value, control = QLabel('masked'), QPushButton('Anzeigen')
        window.toggle_secret(value, 'synthetic-only', control)
        self.assertEqual(value.text(), 'masked')
        with patch('safeincloud_linux.qt_ui.Editor') as editor, patch.object(QMessageBox, 'question') as question:
            window.edit_entry()
            window.edit_entry(True)
            window.delete_entry()
            self.assertFalse(editor.called)
            self.assertFalse(question.called)
        self.assertFalse(window.apply_entry(None, 'Unauthorized', []))
        self.assertFalse(window.persist())
        window.toggle_favorite(True)
        window.refresh_navigation()
        window.refresh_entries()
        self.assertEqual(window.entries.count(), 0)
        with patch('safeincloud_linux.qt_ui.threading.Thread') as worker:
            window.on_sync = lambda _: 'unused'
            window.sync()
            self.assertFalse(worker.called)
        window.on_sync = None
        window.eventFilter(window.password, QKeyEvent(QEvent.KeyPress, Qt.Key_A, Qt.NoModifier))
        self.assertFalse(window.idle.isActive())

    def test_no_access_before_first_login_or_after_wrong_password(self):
        self.assert_access_blocked()
        self.assertFalse(self.window.unlock('wrong'))
        self.assertIsNone(self.window.vault)
        self.assert_access_blocked()

    def test_cli_show_requires_master_password(self):
        from safeincloud_linux.__main__ import main
        arguments = ['safeincloud-linux', str(self.path), '--show', '1']
        for password in ('', 'wrong', 'test-only'):
            with self.subTest(password=password):
                output, errors = io.StringIO(), io.StringIO()
                with patch('sys.argv', arguments), patch('getpass.getpass', return_value=password), redirect_stdout(output), redirect_stderr(errors):
                    result = main()
                if password == 'test-only':
                    self.assertEqual(result, 0)
                    self.assertIn('synthetic-only', output.getvalue())
                else:
                    self.assertEqual(result, 1)
                    self.assertEqual(output.getvalue(), '')
                    self.assertNotIn('synthetic-only', errors.getvalue())

    def test_save_failure_locks_all_actions_and_preserves_recovery(self):
        self.pending_edits()
        vault = self.window.vault
        editor = Editor(self.window, self.window.current_card())
        self.window.editors.append(editor)
        editor.show()
        self.window.copy_value('synthetic-only')
        with patch.object(vault, 'save', side_effect=OSError('synthetic full disk')):
            self.assertFalse(self.window.lock())
            self.assertEqual(self.app.clipboard().text(), '')
            self.assertFalse(editor.isVisible())
            self.assertEqual(editor.inputs[0][1].text(), '')
            self.assertEqual(editor.inputs[0][0]['value'], '')
            self.assert_access_blocked()
            self.assertIs(self.window.vault, vault)
            self.assertTrue(vault.dirty)
            self.assertEqual(vault.cards()[0].get('title'), 'Pending')
            self.assertFalse(self.window.unlock('wrong'))
            self.assert_access_blocked()
            self.assertTrue(self.window.unlock('test-only'))
            self.assertFalse(self.window.locked)
            self.assertEqual(self.window.entries.count(), 1)
            self.assertEqual(self.window.current_card().get('title'), 'Pending')
            self.window.copy_value('unsaved-demo')
            self.assertEqual(self.app.clipboard().text(), 'unsaved-demo')
        self.window.editors.remove(editor)
        editor.deleteLater()
        self.assertTrue(self.window.persist())
        reopened = Vault.open(self.path, 'test-only')
        self.assertEqual(reopened.cards()[0].findtext('field'), 'unsaved-demo')
        reopened.close()
        self.assertTrue(self.window.lock())
        self.assertIsNone(self.window.vault)

    def test_idle_and_close_failures_remain_concealed(self):
        for trigger in ('auto_lock', 'close'):
            with self.subTest(trigger=trigger):
                self.pending_edits()
                with patch.object(self.window.vault, 'save', side_effect=OSError('synthetic failure')):
                    getattr(self.window, trigger)()
                    self.assert_access_blocked()
                    self.assertTrue(self.window.isVisible())
                    self.assertTrue(self.window.vault.dirty)

    def test_pending_sync_lock_failure_cannot_reopen_editor(self):
        self.pending_edits()
        self.window.busy = True
        with patch.object(self.window.vault, 'save', side_effect=OSError('synthetic failure')):
            self.assertFalse(self.window.lock())
            self.assertFalse(self.window.unlock('test-only'))
            self.window.jobs.put((False, 'synthetic failure'))
            self.window.poll()
            self.assertFalse(self.window.busy)
            self.assertFalse(self.window.pending_lock)
            self.assert_access_blocked()
            self.assertTrue(self.window.vault.dirty)

    def test_stale_credential_callbacks_do_not_disclose_after_lock(self):
        self.assertTrue(self.window.unlock('test-only'))
        value = self.window.detail.findChild(CopyValueLabel, 'secretValue')
        reveal = next(b for b in self.window.detail.findChildren(QPushButton) if b.text() == 'Anzeigen')
        self.assertTrue(self.window.lock())
        self.app.clipboard().setText('unrelated')
        value.activated.emit()
        reveal.click()
        self.assertEqual(self.app.clipboard().text(), 'unrelated')
        self.assertEqual(value.text(), '••••••••••••')

    def test_editor_result_cannot_cross_lock_and_reauthentication(self):
        self.pending_edits()
        vault = self.window.vault
        def finish(dialog):
            self.window.lock()
            self.assertTrue(self.window.unlock('test-only'))
            dialog.title_input.setText('Stale editor must not save')
            return QDialog.Accepted
        with patch.object(vault, 'save', side_effect=OSError('synthetic failure')), patch.object(Editor, 'exec', finish):
            self.window.edit_entry()
        self.assertEqual(vault.cards()[0].get('title'), 'Pending')

    def test_delete_confirmation_cannot_cross_lock_and_reauthentication(self):
        self.pending_edits()
        vault = self.window.vault
        def confirm(*_):
            self.window.lock()
            self.assertTrue(self.window.unlock('test-only'))
            return QMessageBox.Yes
        with patch.object(vault, 'save', side_effect=OSError('synthetic failure')), patch.object(QMessageBox, 'question', side_effect=confirm):
            self.window.delete_entry()
        self.assertEqual(len(vault.cards()), 1)
