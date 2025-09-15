# Musing

A music player-server for storing, searching through and listening to your music collection.

> [!NOTE]
> Musing by itself is only a server - any interaction with it requires a standalone client.
> It might sound quite [amusing](https://github.com/alfazet/amusing), but that's the only option there is for now
> (other than writing a client of your own).

## Features
- Play audio files of multiple formats: mp3, aac, flac, wav, aif, ogg (powered by [symphonia](https://github.com/pdeljanov/Symphonia)).
- Manage and query your music colletion with a simple JSON-based protocol.
- Extract metadata (titles, albums, genres, even cover art) from tracks.

## Installation
Install Musing from cargo (`cargo install musing`) or download the source code and build it on your own. For Windows users there's a prebuilt binary available in [Releases](https://github.com/alfazet/musing/releases).

## Usage and configuration
Simply run `musing -m=<MUSIC_DIR>`, where `<MUSIC_DIR>` is the directory where you store your music collection. Musing will then index all files in this directory (and recursively in its subdirectories) and create a music database out of them.

To learn more about all available command-line options, run `musing --help`. To avoid having to specify values at every launch (especially the music directory's path), you can create a `musing.toml` config file, which supports the following keys:
- `port`, to specify the port that Musing will listen on.
- `music_dir`, to specify the music directory's path.
- `playlist_dir`, to specify the path to the directory containing your playlists (.m3u and .m3u8 files).
- `audio_device`, to specify which of your system's audio devices will be the default one used by Musing.
Keep in mind that values supplied with command-line arguments take precedence over those specified in the config file.

As noted earlier, Musing is just a server and so requires a client to interact with it. If you want to build your own client, take a look at the [documentation](./DOCS.md) for an API reference.

## TODO
- [ ] Extend the API.
- [ ] Stream raw PCM data (e.g. for use by music visualizers).
- [ ] Add proxy mode (for use with remote servers).
