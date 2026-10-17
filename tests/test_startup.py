import os
os.environ.setdefault('QT_QPA_PLATFORM', 'offscreen')
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import Mock
from PySide6.QtWidgets import QApplication
from safeincloud_linux.startup import SourceWindow


class StartupTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.app = QApplication.instance() or QApplication([])

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.drive = Mock()
        self.drive.configured.return_value = False
        self.window = SourceWindow(self.temp.name, self.drive)
        self.addCleanup(self.window.close)

    def pump(self, predicate):
        deadline = time.monotonic() + 3
        while not predicate() and time.monotonic() < deadline:
            self.app.processEvents()
        self.assertTrue(predicate())

    def test_discover_unique_database_automatically_opens(self):
        row = {'id':'fake', 'path':'SafeInCloud.db'}
        session = Mock(path=Path(self.temp.name)/'encrypted.db')
        self.drive.discover.return_value = [row]
        self.drive.preferred.return_value = row
        self.drive.open_candidate.return_value = session
        self.window.search()
        self.pump(lambda: self.window.source is not None)
        self.drive.open_candidate.assert_called_once_with(row, self.temp.name)
        self.assertEqual(self.window.source, (str(session.path), session))

    def test_multiple_databases_require_explicit_choice(self):
        rows = [{'id':'a','path':'One.db'}, {'id':'b','path':'Two.db'}]
        self.drive.preferred.return_value = None
        self.window.discovered(rows)
        self.assertEqual(self.window.listing.count(), 2)
        self.drive.open_candidate.assert_not_called()

    def test_search_error_leaves_local_open_available(self):
        self.drive.discover.side_effect = ValueError('connection unavailable')
        self.window.search()
        self.pump(lambda: self.window.local_button.isEnabled())
        self.assertIn('connection unavailable', self.window.status.text())
