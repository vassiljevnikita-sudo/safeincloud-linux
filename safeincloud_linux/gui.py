"""Local Tk viewer: no server, clipboard, telemetry, sync, or database writes."""
import tkinter as tk
from tkinter import messagebox, ttk
from . import cards, load


def run(path):
    app = tk.Tk()
    app.title("SafeInCloud Linux — experimental, read only")
    app.geometry("740x520")
    state = {"cards": []}
    password = tk.StringVar()
    query = tk.StringVar()
    reveal = tk.BooleanVar(value=False)
    ttk.Label(app, text="Open a copy of your database. Compatibility is experimental.").pack(pady=8)
    bar = ttk.Frame(app)
    bar.pack(fill="x", padx=12)
    entry = ttk.Entry(bar, textvariable=password, show="•")
    entry.pack(side="left", fill="x", expand=True)
    listing = tk.Listbox(app, exportselection=False)
    details = tk.Text(app, height=11, wrap="word", state="disabled")

    def clear():
        details.configure(state="normal")
        details.delete("1.0", "end")
        details.configure(state="disabled")

    def refresh(*_):
        reveal.set(False)
        clear()
        listing.delete(0, "end")
        state["filtered"] = [c for c in state["cards"] if query.get().casefold() in c.get("title", "").casefold()]
        for card in state["filtered"]:
            listing.insert("end", card.get("title", "Untitled"))

    def display(*_):
        clear()
        if not listing.curselection():
            return
        card = state["filtered"][listing.curselection()[0]]
        text = "\n".join(f"{f.get('name', '')}: {f.text or '' if reveal.get() else '••••••'}" for f in card.findall("field"))
        if reveal.get():
            text += "\n\n" + card.findtext("notes", "")
        details.configure(state="normal")
        details.insert("1.0", text)
        details.configure(state="disabled")

    def select(*_):
        reveal.set(False)
        display()

    def lock():
        state.clear()
        state["cards"] = []
        password.set("")
        refresh()

    def unlock():
        lock_cards = password.get()
        password.set("")
        lock()
        try:
            state["cards"] = cards(load(path, lock_cards))
            refresh()
        except Exception:
            messagebox.showerror("Cannot open", "Incorrect password, unsupported format, or unreadable database.")
        finally:
            lock_cards = None

    ttk.Button(bar, text="Unlock", command=unlock).pack(side="left")
    ttk.Button(bar, text="Lock", command=lock).pack(side="left")
    ttk.Label(app, text="Search titles").pack()
    ttk.Entry(app, textvariable=query).pack(fill="x", padx=12)
    query.trace_add("write", refresh)
    listing.pack(fill="both", expand=True, padx=12, pady=8)
    listing.bind("<<ListboxSelect>>", select)
    ttk.Checkbutton(app, text="Reveal selected card's fields and notes", variable=reveal, command=display).pack()
    details.pack(fill="both", padx=12, pady=8)
    entry.bind("<Return>", lambda _: unlock())
    entry.focus_set()
    app.mainloop()
    return 0
