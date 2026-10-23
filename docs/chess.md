# Chess against the chat

A game the audience plays together: you play on lichess against a bot account, and on
the bot's turn the chat votes for its move. The move with the most votes is played. Remux
gives the chat (`remux events -f --json`, its `chat` events) and the picture; lichess gives
the game. The bot is a small program of your own, in any language, run as a companion
(`remux companion`), so the TUI starts and stops it with the rest.

This page is the accounts, the program's loop, and the traps, in the order to build them.

## The two accounts

You play from your own lichess account. The chat plays from a second one, a **bot
account**, which only the bot program drives.

1. Create a new lichess account for the bot. It must never have played a game: lichess
   refuses to upgrade an account that has.
2. On that account, create a personal API token at lichess.org/account/oauth/token with
   the scope `bot:play`.
3. Upgrade it to a bot, once, with that token:

   ```sh
   curl -X POST -H "Authorization: Bearer $TOKEN" https://lichess.org/api/bot/account/upgrade
   ```

   It answers `{"ok":true}`. The upgrade cannot be undone.
4. To play from a board of your own rather than lichess's page, your own account needs a
   token too, with `board:play`. Optional.

Keep each token where only you can read it, never in a repository and never on a command
line:

```sh
read -rs t && umask 077 && printf %s "$t" > ~/.config/remux/chess-bot.token; unset t
```

The program reads the file, or an environment variable a companion's `env_file` sets.

## The program's loop

1. `GET https://lichess.org/api/stream/event` with the bot's token: one JSON object a line.
   A `challenge` is accepted with `POST /api/challenge/{id}/accept`; a `gameStart` opens
   the game.
2. `GET /api/bot/game/stream/{gameId}`: the first line is `gameFull` (the starting
   position and `state.moves`, the moves so far in UCI, separated by spaces), every line
   after it a `gameState`. Rebuild the position from the moves and know whose turn it is.
3. On the bot's turn, open a vote: a window of a few seconds (twenty works), timed by the
   program itself.
4. Read `remux events -f --json` and take the `chat` events. Read each message as a move,
   in SAN (`e4`, `Nf3`, `O-O`) or UCI (`e2e4`), with a chess library that knows which moves
   are legal; anything else is not a vote. One vote per person: their last one counts.
5. When the window closes, play the move with the most votes:
   `POST /api/bot/game/{gameId}/move/{uci}`. Nobody voted: open one more window, then play
   a legal move at random.
6. Say what happened where the audience reads it. Either draw it in a window of your own
   that is on the captured screen (whose turn, the leading move, the seconds left), or put
   it in the scene as a text layer (`remux scene layer add text …`). The chat bridge may
   only read, so a line said into the chat may reach nobody.

Challenge the bot from your account, **casual**: a rated game against a bot is refused.

## The traps

- **Chat text is a stranger's.** Never execute it, never put it into a command or a query.
  Only the move a chess library makes of it goes anywhere.
- **One request at a time to lichess.** On a `429`, wait sixty seconds. When a stream
  drops, open it again.
- **`remux events -f` sends what it holds first.** Skip every event whose `seq` is at or
  below the last one you had before you started.
- **One bot program per bot account.** Two would play the same game at once.
- **Start a game from a known state.** A bot started in the middle of a game reads the
  position from `gameFull` and goes on from there.
