import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch


@unittest.skipUnless(os.environ.get("SAFEINCLOUD_GUI_TEST"), "GUI test requires a display and Tk")
class StartupTests(unittest.TestCase):
    def test_drive_connect_list_select_download(self):
        import tkinter as tk
        from safeincloud_linux.startup import choose_source
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / "encrypted.db"
            target.write_bytes(b"\x05\x05\x01demo")
            def exercise(app):
                def widgets(parent):
                    for w in parent.winfo_children():
                        yield w
                        yield from widgets(w)
                all_widgets = list(widgets(app))
                buttons = [w for w in all_widgets if w.winfo_class() == "TButton"]
                listing = next(w for w in all_widgets if w.winfo_class() == "Listbox")
                next(w for w in buttons if w.cget("text") == "Google Drive verbinden").invoke()
                deadline = time.monotonic() + 3
                while listing.size() == 0 and time.monotonic() < deadline:
                    app.update()
                self.assertEqual(listing.get(0), "Backups/SafeInCloud.db")
                listing.selection_set(0)
                with patch.object(app, "quit") as quit_app:
                    next(w for w in buttons if w.cget("text") == "Aus Google Drive öffnen").invoke()
                    deadline = time.monotonic() + 3
                    while not quit_app.called and time.monotonic() < deadline:
                        app.update()
                    self.assertTrue(quit_app.called)
            with patch("safeincloud_linux.startup.GoogleDrive") as cls, patch.object(tk.Tk, "mainloop", exercise):
                cls.return_value.list_databases.return_value = ["Backups/SafeInCloud.db"]
                cls.return_value.download.return_value = target
                self.assertEqual(choose_source(directory), str(target))
                cls.return_value.connect.assert_called_once()
                cls.return_value.download.assert_called_once_with("Backups/SafeInCloud.db", directory)
