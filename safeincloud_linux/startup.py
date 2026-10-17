"""Local file selection and Google Drive login/download, before vault unlocking."""
from pathlib import Path
import queue
import tempfile
import threading
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from .drive import GoogleDrive
from .gui import run


def choose_source(directory):
    app = tk.Tk()
    app.title("SafeInCloud Linux — Datenbank öffnen")
    app.geometry("720x460")
    selected = []
    events = queue.Queue()
    drive = GoogleDrive()
    status = tk.StringVar(value="Lokale Datei öffnen oder eine Kopie aus Google Drive laden.")
    ttk.Label(app, textvariable=status, wraplength=680).pack(padx=16, pady=12)
    buttons = ttk.Frame(app)
    buttons.pack(fill="x", padx=16)
    listing = tk.Listbox(app, exportselection=False)

    def finish(path):
        if path:
            selected.append(str(path))
            app.quit()

    def local():
        finish(filedialog.askopenfilename(parent=app, title="Datenbankkopie öffnen",
               filetypes=[("SafeInCloud-Datenbank", "*.db"), ("Alle Dateien", "*")]))

    def busy(value):
        for button in controls:
            button.configure(state="disabled" if value else "normal")

    def task(work, callback, label):
        busy(True)
        status.set(label)
        def worker():
            try:
                events.put((callback, work(), None))
            except Exception as error:
                events.put((callback, None, str(error)))
        threading.Thread(target=worker, daemon=True).start()

    def listed(paths):
        listing.delete(0, "end")
        for path in paths:
            listing.insert("end", path)
        status.set(f"{len(paths)} Datenbankdatei(en) gefunden." if paths else
                   "Keine .db-Datei gefunden. Die Kopie muss als normale Datei in deinem Drive liegen.")

    def refresh():
        task(drive.list_databases, listed, "Google Drive wird nach .db-Dateien durchsucht …")

    def connect():
        task(drive.connect, lambda _: refresh(), "Bitte im Browser bei Google anmelden. Warte auf die Bestätigung …")

    def download():
        if not listing.curselection():
            messagebox.showinfo("Datei auswählen", "Bitte zuerst eine Datenbank aus der Liste auswählen.", parent=app)
            return
        path = listing.get(listing.curselection()[0])
        task(lambda: drive.download(path, directory), finish, "Verschlüsselte Datenbankkopie wird heruntergeladen …")

    controls = [ttk.Button(buttons, text=text, command=command) for text, command in [
        ("Lokale Datei", local), ("Google Drive verbinden", connect), ("Drive-Dateien laden", refresh)]]
    for button in controls:
        button.pack(side="left", padx=4)
    listing.pack(fill="both", expand=True, padx=16, pady=12)
    open_button = ttk.Button(app, text="Aus Google Drive öffnen", command=download)
    open_button.pack(pady=6)
    controls.append(open_button)
    ttk.Label(app, text="Nur lesender Zugriff. Die temporäre Kopie bleibt verschlüsselt.\n"
              "Safes versteckter App-Ordner ist für andere Anwendungen nicht zugänglich.",
              wraplength=680).pack(pady=10)

    def poll():
        try:
            callback, result, error = events.get_nowait()
        except queue.Empty:
            pass
        else:
            busy(False)
            if error:
                status.set("Vorgang fehlgeschlagen.")
                messagebox.showerror("Google Drive", error, parent=app)
            else:
                callback(result)
        if app.winfo_exists():
            app.after(100, poll)
    app.protocol("WM_DELETE_WINDOW", app.quit)
    app.after(100, poll)
    app.mainloop()
    app.destroy()
    return selected[0] if selected else None


def launch(path=None):
    if path:
        return run(path)
    with tempfile.TemporaryDirectory(prefix="safeincloud-linux-") as directory:
        selected = choose_source(directory)
        if selected:
            return run(selected)
    return 0
