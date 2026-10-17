"""Safe's installed desktop OAuth flow and app-folder Drive protocol.

Original-app client parameters are read locally; they are not distributed here.
"""
import json
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode, quote
from urllib.request import Request, build_opener, HTTPRedirectHandler

from . import MAX_FILE
from .drive import GoogleDrive, DriveError, DriveConflict, DrivePermissionError
from .official_oauth import OfficialOAuth


class OfficialDrive(GoogleDrive):
    def __init__(self, config=None, oauth=None):
        super().__init__(config=config)
        self.oauth = oauth or OfficialOAuth(self.config.parent)

    def configured(self):
        return self.oauth.configured()

    def writable(self):
        # appdata gives read/write to this one application's private folder.
        return self.configured()

    def connect(self, writable=False):
        self.oauth.connect()

    def _token(self):
        return self.oauth.access_token()

    def _get(self, resource, params, max_bytes):
        headers = {"Authorization": "Bearer " + self._token()}
        url = "https://www.googleapis.com/drive/v3/" + resource + "?" + urlencode(params)
        class NoRedirect(HTTPRedirectHandler):
            def redirect_request(self, *_args, **_kwargs):
                return None
        try:
            with build_opener(NoRedirect()).open(Request(url, headers=headers), timeout=60) as response:
                data = response.read(max_bytes + 1)
                if len(data) > max_bytes:
                    raise DriveError("Google-Datei oder Antwort ist zu groß.")
                return data
        except HTTPError as error:
            error.close()
            if error.code in (401, 403):
                raise DrivePermissionError("Google-Anmeldung für den Safe-App-Ordner erforderlich.") from None
            raise DriveError("Google Drive konnte die Safe-Datei nicht laden.") from None
        except (URLError, TimeoutError, OSError):
            raise DriveError("Google-Drive-Verbindung fehlgeschlagen.") from None

    def discover(self):
        rows = []
        page = None
        while True:
            params = {"spaces": "appDataFolder", "q": "trashed = false",
                      "fields": "nextPageToken,files(id,name,size)", "pageSize": "1000"}
            if page:
                params["pageToken"] = page
            try:
                result = json.loads(self._get("files", params, 4 * 1024 * 1024))
                for item in result.get("files", []):
                    name = item.get("name", "")
                    if name.casefold().startswith("safeincloud") and name.casefold().endswith(".db") and 0 <= int(item.get("size", -1)) <= MAX_FILE:
                        rows.append({"id": item["id"], "path": name, "shared": False})
                page = result.get("nextPageToken")
            except DriveError:
                raise
            except (ValueError, TypeError, KeyError):
                raise DriveError("Die Safe-Dateiliste von Google konnte nicht gelesen werden.") from None
            if not page:
                break
        return sorted(rows, key=lambda item: item["path"].casefold())

    def _download_id(self, file_id, destination):
        data = self._get("files/" + quote(file_id, safe=""), {"alt": "media"}, MAX_FILE)
        # destination was exclusively created by the parent, mode 0600.
        with Path(destination).open("wb") as handle:
            handle.write(data)
