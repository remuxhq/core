# ---- the music folder ----
# One folder per genre, the audio files inside are the playlist; never in the
# repository. Where it comes from: StreamBeats (Senpai Records, sync licence),
# whose site links its own Google Drive from every album page:
#   curl -sL https://www.streambeats.com/album/electric/ | grep -o 'drive.google.com[^"]*'
# Downloaded as WAV, kept as MP3 at 192k, named `Artist - Title.mp3`; the
# engine reads the artist off the name. A fetch is a session's work with a
# plan; what stays is the check every file passes before the engine sees it.
.PHONY: music.check
music.check: ## Every file in music/ is one clean MPEG audio stream, decoded whole, plain tags
	@python3 scripts/music-check.py music
