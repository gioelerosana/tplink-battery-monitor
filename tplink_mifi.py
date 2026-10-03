#!/usr/bin/env python3
"""Client minimale per l'API web dei router mobili TP-Link (es. M7350).

Reverse engineered dall'interfaccia web del router (`js/tpweb.min.js`,
`js/libs.min.js`).  L'API e' JSON su due endpoint:

  POST /cgi-bin/auth_cgi   modulo "authenticator" (login)
  POST /cgi-bin/web_cgi    tutti gli altri moduli (es. "status")

Formato delle richieste:
  - moduli senza cifratura:  {"data": "<base64 del JSON>"}
  - richieste cifrate (GDPR): {"data": "<AES-128-CBC base64>",
                               "sign": "<RSA-512 PKCS#1 v1.5 hex>"}

La firma RSA contiene:
  login -> "key=<aesKey>&iv=<aesIv>&h=<md5(admin+pwd)>&s=<seq+len(data)>"
  resto -> "h=<md5(admin+pwd)>&s=<seq+len(data)>"

Le risposte cifrate sono base64 di AES-128-CBC con la stessa key/iv.
"""

from __future__ import annotations

import base64
import hashlib
import json
import secrets
import string
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from typing import Any

from cryptography.hazmat.primitives import padding as sym_padding
from cryptography.hazmat.primitives.asymmetric import padding as asym_padding
from cryptography.hazmat.primitives.asymmetric.rsa import RSAPublicNumbers
from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes

AUTH_CGI = "/cgi-bin/auth_cgi"
WEB_CGI = "/cgi-bin/web_cgi"

MODULES = {
    "authenticator": "authenticator",
    "webServer": "webServer",
    "status": "status",
    "reboot": "reboot",
}

ACTION = {
    "auth_load": 0,
    "auth_login": 1,
    "web_get_feature_list": 5,
    "web_get_without_auth_info": 6,
    "status": 0,
    "reboot": 0,
}

NETWORK_TYPES = {
    0: "Nessun servizio",
    1: "2G (GSM)",
    2: "3G (WCDMA)",
    3: "4G (LTE)",
    4: "3G (TD-SCDMA)",
    5: "CDMA 1x",
    6: "CDMA EVDO",
    7: "4G+ (LTE+)",
}


def format_bytes(num_bytes: float) -> str:
    """Formatta i byte in un'unita' leggibile (base 1024), come l'interfaccia web del router."""
    val = float(num_bytes)
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if abs(val) < 1024.0 or unit == "TB":
            return f"{val:.2f} {unit}"
        val /= 1024.0
    return f"{val:.2f} GB"


def format_speed(bytes_per_sec: float) -> str:
    """Formatta la velocita' in KB/s o MB/s."""
    val = float(bytes_per_sec)
    if val >= 1024.0 * 1024.0:
        return f"{val / (1024.0 * 1024.0):.1f} MB/s"
    return f"{val / 1024.0:.1f} KB/s"


def format_progress_bar(percent: float, length: int = 12) -> str:
    """Restituisce una barra di progresso testuale, es: [████░░░░░░░░] 33.5%."""
    pct = max(0.0, min(100.0, percent))
    filled = int(round((pct / 100.0) * length))
    bar = "█" * filled + "░" * (length - filled)
    return f"[{bar}] {pct:.1f}%"


class TpLinkError(Exception):
    pass


class AuthError(TpLinkError):
    pass


def _b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


def _unb64(text: str) -> bytes:
    return base64.b64decode(text)


@dataclass
class MiFiClient:
    host: str = "192.168.0.1"
    password: str = "admin"
    username: str = "admin"
    timeout: float = 8.0

    _token: str | None = field(default=None, init=False)
    _seq: int = field(default=0, init=False)
    _aes_key: str = field(default="", init=False)
    _aes_iv: str = field(default="", init=False)
    _rsa_nn: str = field(default="", init=False)
    _rsa_ee: str = field(default="", init=False)
    _hash: str = field(default="", init=False)

    def __post_init__(self) -> None:
        if not self.host.startswith(("http://", "https://")):
            self.host = "http://" + self.host
        self.host = self.host.rstrip("/")

    # ------------------------------------------------------------------ HTTP
    def _post(self, path: str, payload: dict[str, Any]) -> str:
        body = json.dumps(payload).encode()
        req = urllib.request.Request(
            self.host + path,
            data=body,
            method="POST",
            headers={
                "Content-Type": "application/json",
                "Referer": self.host + "/",
                "Origin": self.host,
            },
        )
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                return resp.read().decode("utf-8", "replace")
        except urllib.error.HTTPError as exc:  # pragma: no cover
            raise TpLinkError(f"HTTP {exc.code} da {path}") from exc
        except urllib.error.URLError as exc:
            raise TpLinkError(f"Impossibile contattare {self.host}: {exc.reason}") from exc

    # -------------------------------------------------------------- cifratura
    def _aes_encrypt(self, plaintext: str) -> str:
        padder = sym_padding.PKCS7(128).padder()
        data = padder.update(plaintext.encode()) + padder.finalize()
        cipher = Cipher(
            algorithms.AES(self._aes_key.encode()), modes.CBC(self._aes_iv.encode())
        )
        enc = cipher.encryptor()
        return _b64(enc.update(data) + enc.finalize())

    def _aes_decrypt(self, ciphertext_b64: str) -> str:
        raw = _unb64(ciphertext_b64)
        cipher = Cipher(
            algorithms.AES(self._aes_key.encode()), modes.CBC(self._aes_iv.encode())
        )
        dec = cipher.decryptor()
        padded = dec.update(raw) + dec.finalize()
        unpadder = sym_padding.PKCS7(128).unpadder()
        return (unpadder.update(padded) + unpadder.finalize()).decode("utf-8", "replace")

    def _rsa_sign(self, message: str) -> str:
        """Firma il messaggio con RSA-512 a blocchi (max 53 byte ciascuno).

        L'encryptor del firmware (`lb`) spezza la stringa in chunk di
        `e-11` byte e concatena le cifrature esadecimali, perche' una
        singola chiave a 512 bit non puo' cifrare piu' di 53 byte.
        """
        e = int(self._rsa_ee, 16)
        n = int(self._rsa_nn, 16)
        pub = RSAPublicNumbers(e, n).public_key()
        chunk_size = (n.bit_length() // 8) - 11  # 53 per RSA-512
        out = []
        for i in range(0, len(message), chunk_size):
            chunk = message[i : i + chunk_size].encode()
            out.append(pub.encrypt(chunk, asym_padding.PKCS1v15()).hex())
        return "".join(out)

    def _sign(self, data_b64: str, is_login: bool) -> str:
        seq = self._seq + len(data_b64)
        if is_login:
            msg = f"key={self._aes_key}&iv={self._aes_iv}&h={self._hash}&s={seq}"
        else:
            msg = f"h={self._hash}&s={seq}"
        return self._rsa_sign(msg)

    def _encrypt_payload(self, obj: dict[str, Any], is_login: bool) -> str:
        data = self._aes_encrypt(json.dumps(obj, separators=(",", ":")))
        sign = self._sign(data, is_login)
        return json.dumps({"data": data, "sign": sign})

    # ---------------------------------------------------------------- parsing
    def _decode(self, text: str) -> dict[str, Any]:
        """Decodifica una risposta: JSON in chiaro, AES (GDPR) o base64+JSON."""
        text = text.strip()
        try:
            return json.loads(text)
        except json.JSONDecodeError:
            pass
        if self._aes_key:
            try:
                return json.loads(self._aes_decrypt(text))
            except Exception:  # noqa: BLE001
                pass
        return json.loads(_unb64(text).decode("utf-8", "replace"))

    # ------------------------------------------------------------------ login
    def _auth_load(self) -> dict[str, Any]:
        body = json.dumps({"data": _b64(json.dumps(
            {"module": "authenticator", "action": ACTION["auth_load"]}
        ).encode())})
        return self._decode(self._post(AUTH_CGI, json.loads(body)))

    def login(self) -> str:
        info = self._auth_load()
        if info.get("result") not in (0, 1):
            raise AuthError(f"auth_cgi load fallito: {info}")
        nonce = info.get("nonce", "")
        self._rsa_nn = info.get("rsaMod", "")
        self._rsa_ee = info.get("rsaPubKey", "")
        self._seq = int(info.get("seqNum", 0))

        alphabet = string.ascii_letters + string.digits
        self._aes_key = "".join(secrets.choice(alphabet) for _ in range(16))
        self._aes_iv = "".join(secrets.choice(alphabet) for _ in range(16))
        self._hash = hashlib.md5((self.username + self.password).encode()).hexdigest()
        digest = hashlib.md5(
            f"{self.password}:{nonce}".encode()
        ).hexdigest()

        payload = self._encrypt_payload(
            {"module": "authenticator", "action": ACTION["auth_login"], "digest": digest},
            is_login=True,
        )
        resp = self._decode(self._post(AUTH_CGI, json.loads(payload)))
        result = resp.get("result")
        if result == 1:
            raise AuthError("Password del router errata")
        if result != 0:
            raise AuthError(f"Login fallito (result={result}): {resp}")
        self._token = resp.get("token")
        if not self._token:
            raise AuthError(f"Login senza token: {resp}")
        return self._token

    # ---------------------------------------------------------------- richieste
    def call(self, module: str, action: int, data: dict[str, Any] | None = None) -> dict[str, Any]:
        if module != "authenticator" and not self._token:
            self.login()
        obj: dict[str, Any] = {"module": module, "action": action}
        if data:
            obj.update(data)
        if self._token:
            obj["token"] = self._token
        payload = self._encrypt_payload(obj, is_login=False)
        raw = self._post(WEB_CGI if module != "authenticator" else AUTH_CGI, json.loads(payload))
        return self._decode(raw)

    # ---------------------------------------------------------------- comandi
    def get_status(self) -> dict[str, Any]:
        resp = self.call("status", ACTION["status"])
        if resp.get("result") != 0:
            # token scaduto: riprova una volta dopo nuovo login
            self._token = None
            self.login()
            resp = self.call("status", ACTION["status"])
        return resp

    def battery(self) -> dict[str, Any]:
        """Livello batteria.

        Nota: nel firmware il campo si chiama "voltage" ma contiene gia' la
        percentuale (0-100); l'interfaccia web lo usa direttamente per
        scegliere l'icona. Lo esponiamo anche come "level".
        """
        status = self.get_status()
        battery = status.get("battery", {})
        voltage = battery.get("voltage")
        return {
            "connected": battery.get("connected"),
            "charging": battery.get("charging"),
            "level": voltage,
            "voltage": voltage,
            "model": status.get("deviceInfo", {}).get("model"),
        }

    def data_usage(self, status: dict[str, Any] | None = None) -> dict[str, Any]:
        """Statistiche sul consumo dati (giga consumati, totale piano e rimanenti)."""
        if status is None:
            status = self.get_status()
        wan = status.get("wan", {})
        try:
            total_bytes = float(wan.get("totalStatistics") or 0)
        except (ValueError, TypeError):
            total_bytes = 0.0
        try:
            daily_bytes = float(wan.get("dailyStatistics") or 0)
        except (ValueError, TypeError):
            daily_bytes = 0.0
        try:
            limit_bytes = float(wan.get("limitation") or 0)
        except (ValueError, TypeError):
            limit_bytes = 0.0

        has_limit = bool(wan.get("enableDataLimit")) and limit_bytes > 0
        remaining_bytes = max(0.0, limit_bytes - total_bytes) if has_limit else None
        usage_percent = (total_bytes / limit_bytes * 100.0) if has_limit and limit_bytes > 0 else None

        net_code = wan.get("networkType")
        net_name = NETWORK_TYPES.get(net_code, f"Sconosciuto ({net_code})" if net_code is not None else "N/A")

        return {
            "total_bytes": total_bytes,
            "total_formatted": format_bytes(total_bytes),
            "daily_bytes": daily_bytes,
            "daily_formatted": format_bytes(daily_bytes),
            "limit_bytes": limit_bytes,
            "limit_formatted": format_bytes(limit_bytes) if has_limit else "Nessun limite",
            "has_limit": has_limit,
            "remaining_bytes": remaining_bytes,
            "remaining_formatted": format_bytes(remaining_bytes) if remaining_bytes is not None else "N/A",
            "usage_percent": usage_percent,
            "progress_bar": format_progress_bar(usage_percent) if usage_percent is not None else "",
            "operator": wan.get("operatorName", ""),
            "network_type_code": net_code,
            "network_type": net_name,
            "signal_strength": wan.get("signalStrength", 0),
            "signal_percent": int((wan.get("signalStrength", 0) / 4.0) * 100) if wan.get("signalStrength") is not None else 0,
            "rx_speed": float(wan.get("rxSpeed") or 0),
            "rx_speed_formatted": format_speed(float(wan.get("rxSpeed") or 0)),
            "tx_speed": float(wan.get("txSpeed") or 0),
            "tx_speed_formatted": format_speed(float(wan.get("txSpeed") or 0)),
        }

    def summary(self) -> dict[str, Any]:
        """Riepilogo completo per companion app / widget."""
        status = self.get_status()
        battery_info = status.get("battery", {})
        voltage = battery_info.get("voltage")
        device_info = status.get("deviceInfo", {})
        return {
            "model": device_info.get("model", "M7350"),
            "firmware": device_info.get("firmwareVer", ""),
            "battery": {
                "connected": battery_info.get("connected"),
                "charging": battery_info.get("charging"),
                "level": voltage,
                "voltage": voltage,
            },
            "data": self.data_usage(status),
            "connected_devices": status.get("connectedDevices", {}).get("number", 0),
        }

    def reboot(self) -> dict[str, Any]:
        return self.call("reboot", ACTION["reboot"])


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="Client API TP-Link MiFi")
    parser.add_argument("--host", default="192.168.0.1")
    parser.add_argument("--password", required=True)
    parser.add_argument("command", choices=["battery", "status", "data", "summary", "reboot"])
    args = parser.parse_args()

    client = MiFiClient(host=args.host, password=args.password)
    client.login()
    if args.command == "battery":
        print(json.dumps(client.battery(), indent=2, ensure_ascii=False))
    elif args.command == "status":
        print(json.dumps(client.get_status(), indent=2, ensure_ascii=False))
    elif args.command == "data":
        print(json.dumps(client.data_usage(), indent=2, ensure_ascii=False))
    elif args.command == "summary":
        print(json.dumps(client.summary(), indent=2, ensure_ascii=False))
    elif args.command == "reboot":
        print(json.dumps(client.reboot(), indent=2, ensure_ascii=False))
