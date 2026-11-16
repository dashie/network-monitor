# Notifications — runbook

How to declare, enable, verify, and repair a notification channel of `network-monitor`. The
behaviour is stated in [PRODUCT.md](./PRODUCT.md#functional-behaviour) (`FR-25`, `FR-28`,
`FR-29`) and in [USER-GUIDE.md](./USER-GUIDE.md#notifications); this page is the operating
procedure and the list of failure modes.

Three channel types exist: Telegram, a webhook, and a log file. A channel is declared once in
`[[notification]]` and enabled by name in a `notify` list; it has no scope of its own.

## What the tool needs

| Type | Required | Optional |
|---|---|---|
| `telegram` | `name`, `bot_token`, `chat_id` | `stabilize_seconds` |
| `webhook` | `name`, `url` | `token` (requires `https`), `stabilize_seconds` |
| `logfile` | `name`, `filename` | `logger`, `level`, `format`, `stabilize_seconds` |

## 1. Declare the channel and enable it

```toml
notify = ["telegram"]            # global events, and the default for every target

[[notification]]
name = "telegram"
type = "telegram"
bot_token = "123456789:replace-me"
chat_id = "-1001234567890"

[[targets]]
ip = "192.168.1.1"
notify = ["telegram"]            # replaces the root list for this target
```

The root `notify` list enables a channel for global events — start, stop, reload, channel
start — and as the default for every target. A target's `notify` list replaces it; an absent
key inherits it; `notify = []` silences the target entirely. A channel that is declared but
enabled nowhere is a configuration error, so a forgotten declaration is loud.

## 2. Telegram

### Create the bot

1. Open Telegram and start a chat with **@BotFather**.
2. Send `/newbot`, choose a display name and a user name ending in `bot`.
3. Copy the token it answers with, of the form `123456789:AAE…`. It is the only time the
   token is shown; `/token` in BotFather shows it again if it is lost.
4. If the token leaks, send `/revoke` to BotFather and write the new one into the
   configuration. The old token stops working immediately, so the channel goes quiet
   until the file is updated and the monitor restarted.

### Prepare the chat

- **Channel (recommended for one-way alerts).** Create it, open *Administrators*, add the
  bot, and make it an **administrator**. A bot that is a member but not an administrator
  cannot post, and the Bot API answers `400 Bad Request: not enough rights`.
- **Group.** Add the bot as a member; a group allows posting without administrator rights
  unless the group restricts members.
- **Private chat.** Start the bot and send it any message, otherwise it cannot write.

### Find the `chat_id`

Post one message in the channel as a human, then read the updates:

```bash
curl -sS "https://api.telegram.org/bot<token>/getUpdates" | \
  python3 -c 'import json,sys; d=json.load(sys.stdin); print([u.get("channel_post", u.get("message", {})).get("chat", {}) for u in d["result"]])'
```

- A channel post appears in `channel_post.chat.id` and starts with `-100`.
- A private chat has a positive `chat.id`.
- An empty `result` means the bot has seen no message since it was created; post again.

`getUpdates` returns nothing while a webhook is set. If the bot was ever attached to a
webhook, remove it with `curl "https://api.telegram.org/bot<token>/deleteWebhook"` and
post again.

### Test before touching the service

```bash
TOKEN="123456789:replace-me"
CHAT="-1001234567890"

curl -sS "https://api.telegram.org/bot${TOKEN}/getMe"
curl -sS -X POST "https://api.telegram.org/bot${TOKEN}/sendMessage" \
  -d "chat_id=${CHAT}" \
  -d "text=network-monitor: channel test" \
  -d "disable_web_page_preview=true"
```

A working channel answers `{"ok":true,…}`. A failing one answers
`{"ok":false,"error_code":…,"description":"…"}`; the description is the sentence the monitor
puts on its `WARN` line.

## 3. Webhook

The endpoint receives one `POST` per event, with `Content-Type: application/json;
charset=utf-8`, and `Authorization: Bearer <token>` when a token is configured. A token
requires an `https` URL, so it is never sent in clear.

```bash
curl -sS -X POST "https://automation.example.test/hooks/network-monitor" \
  -H "Authorization: Bearer replace-me" \
  -H "Content-Type: application/json" \
  -d '{"schema":1,"event":"target_down","target":{"key":"icmp://1.1.1.1"},"state":"down"}'
```

Any `2xx` verifies the endpoint. The tool treats `429`, `5xx`, and a transport failure as
retryable; a redirect is not followed, and any other `4xx` is not retried. The body your
endpoint receives is one line:

```json
{"schema":1,"event":"target_down","occurred_at":"2025-11-24T16:00:00.000Z","target":{"key":"icmp://1.1.1.1","label":"cloudflare-dns"},"state":"down","cause":{"code":"ICMP.NO_REPLY","message":"no reply within 2000 ms"}}
```

```toml
[[notification]]
name = "hook"
type = "webhook"
url = "https://automation.example.test/hooks/network-monitor"
token = "replace-me"
```

The token is read from the file only and printed as `<redacted>` by `--show-config`. A
webhook is named in a log line and in an error as `scheme://host[:port]` only, because the
path may itself be a secret.

## 4. Log file

```toml
[[notification]]
name = "audit"
type = "logfile"
filename = "/var/log/network-monitor/audit.log"
logger = "notification"
level = "info"
format = "jsonl"
```

The directory must exist: the tool creates the file, never the directory. The file is
appended to with mode `0600` on Unix and is never rotated by the tool — use `logrotate` or
the platform's mechanism.

```bash
sudo install -d -m 750 -o network-monitor /var/log/network-monitor
tail -f /var/log/network-monitor/audit.log
```

`format = "text"` writes the diagnostic line; `format = "jsonl"` writes one compact JSON
document per line, the webhook body plus `"logger":"<logger>"`. `level` is the most verbose
level admitted, so `error` hides warnings and information.

## 5. Verify

```bash
network-monitor --show-config | grep -A4 '\[\[notification\]\]'   # secrets read <redacted>
```

| Check | Expected |
|---|---|
| Start | One `CHANNEL` line per channel — the log file, the chat, the webhook — and one `READY` on the channels the root list enables |
| First probe after the restart | One `UP` per answering target, after `stabilize_seconds` |
| `tcp` target on a closed port | One `DOWN` at once, with `TCP.REFUSED` and the instant |
| Port reopened | One `UP` after the window, no message before |
| Drop and recovery inside the window | No message |
| A reload | One `CONFIG` message, applied or rejected with the reason |
| `--log-level info` | `notification <event> sent` on success, `WARN notification attempt …` on failure |

## Delivery failure modes

### Telegram

The client posts to `https://api.telegram.org` only, with the roots compiled into the
binary. There is no proxy setting and no way to disable certificate verification.

| Answer | Meaning | Retried |
|---|---|---|
| `200` with `"ok":true` | Delivered | — |
| `400 Bad Request: chat not found` | Wrong `chat_id`, or the bot was removed from the chat | no |
| `400 Bad Request: not enough rights` | The bot is not an administrator of the channel | no |
| `401 Unauthorized` | The token was revoked or mistyped | no |
| `403 Forbidden: bot was blocked by the user` | The recipient blocked the bot | no |
| `429 Too Many Requests` | Rate limit; the `parameters.retry_after` states the wait | yes, five attempts |
| `5xx` | Telegram is unavailable | yes, five attempts |
| Connection refused, timed out, DNS failure | No route to `api.telegram.org` | yes, five attempts |

### Webhook

| Answer | Meaning | Retried |
|---|---|---|
| `2xx` | Delivered | — |
| `3xx` | A redirect, which is never followed | no |
| `400`, `401`, `403`, `404`, … | The endpoint refused the document or the token | no |
| `429` | Rate limit | yes, five attempts |
| `5xx` | The endpoint is unavailable | yes, five attempts |
| Connection refused, timed out, DNS failure | No route to the URL | yes, five attempts |

### Log file

| Symptom | Meaning | Retried |
|---|---|---|
| `cannot open … for append` at startup | The file or its directory is not writable | no; the process does not start |
| `cannot write …` on a line | The filesystem is full or the file was removed | no; one `WARN` line, other channels unaffected |
| No line for an event | The event is below the channel's `level` | not a failure |

After the fifth failed attempt the message is dropped and the log states `notification
lost: …`. Nothing is queued for later: the event is already in the database, and the rule
"one `down` per incident" still holds, because the state machine records the attempt, not
the delivery.

## Reading the log

```text
2026-11-14T12:34:56.123Z WARN  notification [icmp://1.1.1.1] notification attempt 1/5 failed: telegram answered 401: Unauthorized
2026-11-14T12:34:56.124Z WARN  notification [icmp://1.1.1.1] notification lost: telegram answered 401: Unauthorized
2026-11-14T12:34:57.001Z INFO  notification [icmp://1.1.1.1] target_up sent
```

Every line names its logger (`monitor` or `notification`) and, for a target event, the
target. A failure line names the host and the status, and the `description` of the answer
when there is one; it never contains a token or a URL path.

## Recovering

- **The token leaked.** `/revoke` in BotFather, write the new token, restart. Assume anyone
  with the token could post to the chat until the revocation. A leaked webhook token is
  replaced at the endpoint and in the file; restart.
- **A channel must change.** Change its declaration and restart; a reload does not move a
  channel. A changed notification configuration is reported and applied at the next start.
- **A target must be silenced.** Give it `notify = []`; the root list still serves the
  others.
- **A channel must be silenced without editing targets.** Take its name out of the root list
  and out of every target's list, or delete the declaration: a channel that nothing enables is
  legal, so the monitor starts and logs one `WARN` line naming it, and the channel receives only
  its own `CHANNEL` line. A log file channel can instead keep its declaration and raise `level`
  to `error`, which hides warnings and information.
- **The process refuses to start with `unknown notification channel`** — a `notify` list
  names a channel that is not declared. `--show-config` prints both lists.
- **`WARN notification channel 'x' is declared but nothing enables it`** — the channel is in no
  list, so it receives nothing. Add `x` to the root list or to a target's list, or remove the
  declaration; the monitor runs either way.
- **The process refuses to start with `both write to …`** — two channels share one file,
  URL or chat. Declare one channel per destination.
