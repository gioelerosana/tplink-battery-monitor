# Reverse-engineered TP-Link MiFi HTTP API

This document describes the undocumented JSON API served by TP-Link mobile
Wi-Fi routers (MiFi). It was derived by reading the JavaScript the router
serves to its own web UI (`js/tpweb.min.js`, `js/settings.min.js`,
`js/libs.min.js`) and by probing a **TP-Link M7350 (EU) v9.0**, firmware
`9.0.5`, over the LAN. No browser is involved at runtime.

> The device is a settings database with a web page in front of it. Every
> screen is a `module` + `action` call, and the response is JSON.

## Endpoints

| Endpoint | Used for |
| --- | --- |
| `POST /cgi-bin/auth_cgi` | the `authenticator` module (login) |
| `POST /cgi-bin/web_cgi` | every other module (e.g. `status`) |

Both are plain HTTP on port 80. The web UI sends a `Referer`/`Origin` header;
some firmwares reject requests without them, so it is safer to always send
them.

## Request envelope

Every request body is JSON containing `module` and `action`. Depending on the
module, the body is wrapped in one of two ways (see `encodeData` in
`js/tpweb.min.js`):

### 1. Plain base64 wrapper

Used for modules that are not the class of "authenticated web CGI" calls
(notably the `authenticator` *load* action and most `webServer` actions):

```json
{ "data": "<base64 of the JSON {module, action, ...}>" }
```

### 2. Encrypted (GDPR) envelope

Used when `supportGDPR` is true (which it is on this firmware) for the login
and for authenticated modules such as `status`:

```json
{ "data": "<AES-128-CBC(PKCS#7) of the JSON, base64>",
  "sign": "<RSA-512 PKCS#1 v1.5 signature, hex>" }
```

## Module / action vocabulary (subset)

| Module | Actions |
| --- | --- |
| `authenticator` | `load: 0`, `login: 1`, `getAttempt: 2`, `logout: 3`, `update: 4` |
| `webServer` | `getLang: 0`, `setLang: 1`, `keepAlive: 2`, `unsetDefault: 3`, `getModuleList: 4`, `getFeatureList: 5`, `getWithoutAuthInfo: 6` |
| `status` | `getStatus: 0` |
| `reboot` | `reboot: 0`, `powerOff: 1` |

The full vocabulary mirrors the one documented in the
[`tp-link-m7200-api`](https://github.com/mt-ks/tp-link-m7200-api) project,
whose device family shares this API shape.

## Authentication flow (`supportGDPR = true`)

1. **Handshake.** `auth_cgi` with `{"module":"authenticator","action":0}`
   (base64-wrapped) returns:

   ```json
   { "authedIP": "0.0.0.0",
     "nonce": "-UHg6ExPfd3Jb3cO",
     "rsaPubKey": "010001",
     "rsaMod": "F094...7673",
     "seqNum": 1059672589,
     "result": 1 }
   ```

   `rsaMod` is `n`, `rsaPubKey` is `e` (hex), `seqNum` seeds the signature.

2. **Key material.** The client draws a random AES-128 key and IV (16 ASCII
   characters each) and computes `hash = MD5("admin" + password)`.

3. **Login payload.** The digest is `MD5(password + ":" + nonce)`. It is sent
   inside the JSON `{"module":"authenticator","action":1,"digest":"..."}`,
   which is AES-128-CBC (PKCS#7) encrypted and signed.

4. **Signature.** RSA uses a **512-bit** modulus, which can encrypt at most
   `64 - 11 = 53` bytes per block. The firmware therefore splits the signature
   string into chunks of 53 bytes, encrypts each with PKCS#1 v1.5, and
   concatenates the hex blocks (`lb` in `js/libs.min.js`). The signed string
   differs for login vs. normal requests:

   - login: `key=<aesKey>&iv=<aesIv>&h=<hash>&s=<seq + len(data)>`
   - other: `h=<hash>&s=<seq + len(data)>`

   where `data` is the base64 AES ciphertext and `seq` is `seqNum`.

5. **Session.** The login response is base64 of AES-128-CBC with the same
   key/IV; decrypting it yields `{"result":0,"token":"..."}`. The `token` is
   added to every subsequent request's JSON object.

Reimplementing the signature correctly requires the chunking step; a naive
single RSA-512 encryption of the full string fails because the message exceeds
53 bytes.

## Response decoding

A response may be, in order of likelihood:

1. **Plain JSON** (e.g. errors such as `{"result":-4}` for an unauthenticated
   `status` call).
2. **Base64 of AES-128-CBC ciphertext** for encrypted calls (login and
   authenticated modules).
3. **Base64 of a JSON object** for the plain-wrapper modules
   (`getFeatureList`, `getWithoutAuthInfo`, ...).

A robust client tries these in turn (`_decode` / `decode` in the sources).

## The `battery` field

`status` returns a large object; the battery sub-object is:

```json
"battery": { "connected": true, "charging": false, "voltage": 98 }
```

**`voltage` is not a voltage: it is the battery percentage (0–100).** The web
UI (`js/settings.min.js`) feeds it straight into an icon selector:

```js
c = e > 95 ? 100 : e >= 90 ? 90 : e >= 80 ? 80 : ... // icon level
```

So `voltage: 98` means 98%. `connected` reports whether a battery is present;
`charging` whether the charger is connected. Both implementations rename
`voltage` to `level` (while keeping the raw field too).

Useful neighbours in the same response: `deviceInfo.model`,
`deviceInfo.firmwareVer`, `wan.networkType`, `wan.signalStrength`,
`connectedDevices.number`.

## Notes and references

- The plain-wrapper modules leak a lot of unauthenticated information,
  including the model (`getWithoutAuthInfo`) and the feature list
  (`getFeatureList`, which reveals `supportGDPR`).
- The `/cgi_gdpr` + `data`/`sign` text protocol used by other TP-Link
  firmwares is related but distinct; this device uses the JSON `web_cgi`
  dialect described above.
- Prior art / related projects:
  - https://github.com/mt-ks/tp-link-m7200-api
  - https://github.com/0xf15h/tp_link_gdpr
