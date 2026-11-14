# Telegram notifications — runbook

How to enable, verify, and repair the optional Telegram channel of `network-monitor`.
The behaviour itself is stated in [PRODUCT.md](./PRODUCT.md#functional-behaviour)
(`FR-25`) and in [USER-GUIDE.md](./USER-GUIDE.md#telegram-notifications); this page is
the operating procedure and the list of failure modes.

## What the tool needs

| Value | Where it comes from |
|---|---|
| `bot_token` | BotFather, when the bot is created |
| `chat_id` | The chat or channel that receives the messages |

One bot serves one `chat_id` per process. The section is global: every target reports to
the same chat, and a target cannot be silenced on its own.

## 1. Create the bot

1. Open Telegram and start a chat with **@BotFather**.
2. Send `/newbot`, choose a display name and a user name ending in `bot`.
3. Copy the token it answers with, of the form `123456789:AAE…`. It is the only time the
   token is shown; `/token` in BotFather shows it again if it is lost.
4. If the token leaks, send `/revoke` to BotFather and write the new one into the
   configuration. The old token stops working immediately, so the channel goes quiet
   until the file is updated and the monitor restarted.

## 2. Prepare the channel

- **Channel (recommended for one-way alerts).** Create it, open *Administrators*, add the
  bot, and make it an **administrator**. A bot that is a member but not an administrator
  cannot post, and the Bot API answers `400 Bad Request: not enough rights`.
- **Group.** Add the bot as a member; a group allows posting without administrator rights
  unless the group restricts members.
- **Private chat.** Start the bot and send it any message, otherwise it cannot write to
  the chat.

## 3. Find the `chat_id`

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

## 4. Test the channel before touching the service

```bash
TOKEN="123456789:replace-me"
CHAT="-1001234567890"

curl -sS "https://api.telegram.org/bot${TOKEN}/getMe"
curl -sS -X POST "https://api.telegram.org/bot${TOKEN}/sendMessage" \
  -d "chat_id=${CHAT}" \
  -d "text=network-monitor: channel test" \
  -d "disable_web_page_preview=true"
```

A working channel answers `{"ok":true,…}`. A failing one answers `{"ok":false,"error_code":…,
"description":"…"}`; the description is the sentence the monitor puts on its `WARN` line.

## 5. Enable it in the configuration

```toml
[telegram]
bot_token = "123456789:replace-me"
chat_id = "-1001234567890"
stabilize_seconds = 60
```

Then:

```bash
chmod 600 network-monitor.toml        # the file now holds a credential
network-monitor --show-config | grep -A2 telegram   # token must read <redacted>
sudo systemctl restart network-monitor                # or: docker compose restart
```

`--show-config` prints the section with the token replaced by `<redacted>`, so the output
is safe to paste into an issue. A changed section is not applied by the reload: the
monitor logs `telegram settings changed; restart the monitor to use them` and keeps the
running settings until the restart.

## 6. Verify

| Check | Expected |
|---|---|
| First probe after the restart | One `UP` message per answering target, after `stabilize_seconds` |
| `tcp` target on a closed port | One `DOWN` message at once, with `TCP.REFUSED` and the instant |
| Port reopened | One `UP` message after the window, no message before |
| Drop and recovery inside the window | No message |
| `--log-level info` | `notifications` lines only on failure; `INFO … notification sent` on success |

## Delivery failure modes

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

After the fifth failed attempt the message is dropped and the log states
`notification lost: …`. Nothing is queued for later: the event is already in the
database, and the rule "one `down` per incident" still holds, because the state machine
records the attempt, not the delivery.

## Reading the log

```text
2026-11-14T12:34:56.123Z WARN  notification attempt 1/5 failed: telegram answered 401: Unauthorized
2026-11-14T12:34:56.124Z WARN  notification lost: telegram answered 401: Unauthorized
2026-11-14T12:34:57.001Z INFO  [cloudflare-dns] notification sent
```

A failure line names the host and the status, and the `description` of the answer when
there is one. It never contains the request path, so it never contains the token.

## Recovering

- **The token leaked.** `/revoke` in BotFather, write the new token, restart. Assume
  anyone with the token could post to the channel until the revocation.
- **The channel must change.** Write the new `chat_id` and restart; a reload does not
  move the channel.
- **The channel must be silenced.** Remove the `[telegram]` section and restart. The
  database keeps the full history, and the page and `query` are unaffected.
