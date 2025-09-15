# Musing: Docs

Musing listens for incoming connections on `localhost:PORT`, where `PORT` is 2137 by default, You can specify any other port either by a command-line argument or by an entry in the config file.

## Protocol
Musing exchanges data with clients through a TCP socket.
Every message must consist of the following:
- Exactly 4 bytes representing a 32-bit unsigned integer `N` (in big-endian).
- Then, exactly `N` bytes representing a string that parses to a JSON object.
Because every message in the Musing protocol is a JSON object at its core, whenever we write "a message/request contains the key `foo`" we mean that its JSON object contains the key `foo`.

After a client initiates the connection, it receives a message containing exactly one key `version` (the protocol's version is not relevant for now, but might be in the future when a breaking change occurs in the API).

Then, the client is free to send requests to Musing. Every request must contain a `kind` key (which allows Musing to distinguish endpoints) and zero or more additional keys specifying some additional arguments specific to any given request kind. All available requests are described in the next section.

Musing responds to every request with a response, which always contains a `status` key with a value of either `ok` or `err`. If `status` is `err`, then there will be a `reason` key present with a string value which describes why the request failed. Beyond that, responses may contain more keys specyfing details related to the given request. All responses are described in detail in the next section (with the `status`/`reason` keys ommitted for brevity). If a request doesn't have its response prototype listed, that means its response contains only the `status`/`reason` keys.

## Available requests

### ls
```json
{
    "kind": "ls",
    "dir": string,
}
```

Returns paths of all songs located in `dir`. The path can either be absolute or relative to the directory where the database is rooted. Only songs contained in the Musing database are taken into account. Returned paths are always absolute.
Should be used only with untagged/badly tagged music collections. With properly tagged collections using `select` will be more convenient.

Response:
```json
{
    "paths": array of strings,
}
```

Example request:
```json
{
    "kind": "ls",
    "dir": "/some/path",
}
```
Example response:
```json
{
    "status": "ok",
    "paths": ["/some/path/foo.mp3", "/some/path/bar.wav"],
}
```

### metadata
```json
{
    "kind": "metadata",
    "paths": array of strings,
    "tags": array of strings,
}
```
or
```json
{
    "kind": "metadata",
    "paths": array of strings,
    "all_tags": bool,
}
```

Returns objects containing key-value pairs representing the metadata of songs located in `paths`. If `tags` is specified, returns only the values corresponding to the provided tags. If `all_tags` is specified and true, returns values of all tags supported by Musing (you can find a list of supported tags at the end of these docs).

Response:
```json
{
    "metadata": array of objects,
}
```

Example request:
```json
{
    "kind": "metadata",
    "paths": ["/some/song.mp3", "/another/song.m4a"],
    "tags": ["artist"],
}
```
Example response:
```json
{
    "status": "ok",
    "metadata": [{"artist": "Foo Bar"}, {"artist": "Baz Qux"}],
}
```

### select
```json
{
    "kind": "select",
    "tags": array of strings,
    "filters": array of objects,
    "group_by": array of strings,
    "comparators": array of objects,
}
```

Returns paths and values of `tags` of those songs, which satisfy each of the `filters`. The results are grouped by the values of tags in `group_by` and sorted by `comparators`.

A filter is a JSON object with the following structure:
```json
{
    "kind": "regex",
    "tag": string,
    "regex": string,
}
```
As the name suggests, it allows only songs whose value of `tag` matches the regular expression `regex` to "pass through". If a song has no defined value for `tag`, it doesn't pass the filter. Regexes are parsed by the `regex` crate, so a reference of their syntax is available [here](https://docs.rs/regex/latest/regex/#syntax).

A comparator in a JSON object with the following structure:
```json
{
    "tag": string,
    "order": string,
}
```
Its job is to sort the response values according to the value of `tag`. The order is determined by the `order` key, whose only valid values are `"ascending"` and `"descending"`.

Response:
```json
{
    "values": array of objects,
}
```

Example request:
```json
{
    "kind": "select",
    "tags": ["tracktitle"],
    "filters": [
        {
            "kind": "regex",
            "tag": "albumartist",
            "regex": "^M.*",
        },
        {
            "kind": "regex",
            "tag": "album",
            "regex": "^M.*",
        },
    ],
    "group_by": ["album"],
    "comparators": [
        {
            "tag": "tracknumber",
            "order": "descending",
        },
    ],
}
```
Example response:
```json
{
    "status": "ok",
    "values": [
        {
            "album": "Metallica",
            "data": [
                ["Enter Sandman", "tracks/01_enter_sandman.mp3"],
                ["Sad but True", "tracks/02_sad_but_true.mp3"],
                ...
            ]
        },
        {
            "album": "Master of Puppets",
            "data": [
                ["Battery", "tracks/01_battery.mp3"],
                ["Master of Puppets", "tracks/02_master_of_puppets.mp3"],
                ...
            ]
        },
    ],
}
```

### update
```json
{
    "kind": "update",
}
```

Updates the music database, that is adds any files that have been created since the previous update, removes songs whose files don't exist anymore and re-adds songs whose metadata has changed.

### volume
```json
{
    "kind": "volume",
    "delta": integer,
}
```

Changes the volume by `delta` units. The resulting volume is clamped between 0 and 100.

### seek
```json
{
    "kind": "seek",
    "seconds": integer,
}
```

Seeks the audio by `seconds` seconds, backwards if the value is negative, forwards otherwise.

### speed
```json
{
    "kind": "speed",
    "delta": integer,
}
```

Changes the playback speed by `delta` percentage points. The resulting speed is clamped between 25 and 400.

### gapless
```json
{
    "kind": "gapless",
}
```

Toggles gapless playback.

### pause
```json
{
    "kind": "pause",
}
```

Pauses the playback. Does nothing when playback is stopped.

### resume
```json
{
    "kind": "resume",
}
```

Resumes the playback. Does nothing when playback is stopped.

### toggle
```json
{
    "kind": "toggle",
}
```

Toggles the playback. Does nothing when playback is stopped.

### stop
```json
{
    "kind": "stop",
}
```

Stops the playback.

### addqueue
```json
{
    "kind": "addqueue",
    "paths": array of strings
    "pos": integer (optional),
}
```

Adds songs from `paths` to the queue, starting at position `pos` (zero-indexed). Appends songs to the end if `pos` is not specified or invalid.

### play
```json
{
    "kind": "play",
    "id": integer,
}
```

Plays the song present in the queue with id equal to `id`.

### removequeue
```json
{
    "kind": "removequeue",
    "ids": array of integers,
}
```

Removes songs with ids `ids` from the queue.

### clearqueue
```json
{
    "kind": "clearqueue",
}
```

Clears the queue (removes all songs from it).

### next
```json
{
    "kind": "next",
}
```

Plays the next song (the next song is determined by the queue's playback mode). If there is no next song, stops the playback.

### previous
```json
{
    "kind": "previous",
}
```

Plays the previous song from the queue. If there is no previous song, stops the playback.

### modesingle
```json
{
    "kind": "modesingle",
}
```

Switches the queue into single mode: the playback stops after a song is finished.

### moderandom
```json
{
    "kind": "moderandom",
}
```

Switches the queue into random mode: the next song will be chosed from a pool of those enqueued songs that haven't been played yet. After the pool is exhausted, it's regenerated with every song from the queue.

### modesequential
```json
{
    "kind": "modesequential",
}
```

Switches the queue into sequential (the default) mode: songs are played one after another in order of their positions.

### state
```json
{
    "kind": "state",
}
```

Responds with information about the current state of Musing, in particular the response contains:
- the queue (as an array of entries, each entry containing the id and path of the song)
- the (zero-indexed) position in the queue of the current song (or `null` if playback is stopped)
- the base64-encoded cover art of the current song (if available)
- the playback state (playing/paused/stopped)
- the playback mode (single/random/sequential)
- the "gaplessness" of playback
- the volume
- the playback speed
- the timer (an object containing the duration of the current song as well as how many seconds elapsed since it started)
- the list of known playlists
- the list of audio devices (and whether they're disabled/enabled)

In order to prevent sending redundant data, the response is "delta-encoded" i.e. every client receives only the keys whose values have changed since the last time it requested `state`. The first response to any given client will always contain the full state.

Response:
```json
{
    "queue": array of objects,
    "current": integer or null,
    "cover_art": string,
    "playback_state": string,
    "playback_mode": string,
    "gapless": bool,
    "volume": integer,
    "speed": integer,
    "timer": object,
    "playlists": array of strings,
    "devices": array of objects,
}
```

Example request:
```json
{
    "kind": "state",
}
```
Example response:
```json
{
    "status": "ok",
    "queue": [{"id": 2, "path": "/some/song.mp3"}, {"id": 4, "path": "/another/song.m4a"}],
    "current": 1,
    "cover_art": "somebase64encodeddataxyz",
    "playback_state": "paused",
    "playback_mode": "random",
    "gapless": false,
    "volume": 60,
    "speed": 100,
    "timer": {"duration": 234, "elapsed": 100},
    "playlists": ["/playlist/dir/abc.m3u"],
    "devices": [{"device": "pipewire", "enabled": true}],
}
```

### disable
```json
{
    "kind": "disable",
    "device": string,
}
```

Disables the given audio device.

### enable
```json
{
    "kind": "enable",
    "device": string,
}
```

Enables the given audio device.

### addplaylist
```json
{
    "kind": "addplaylist",
    "playlist": string,
    "song": string,
}
```

Appends the `song` to `playlist` (an .m3u or .m3u8 file).

### listsongs
```json
{
    "kind": "listsongs",
    "playlist": string,
}
```

Returns an array containing paths of all songs in the `playlist` file. Paths are relative to the database's root directory.

Response:
```json
{
    "songs": array of strings
}
```

Example request:
```json
{
    "kind": "listsongs",
    "playlist": "/playlist/dir/foobar.m3u",
}
```
Example response:
```json
{
    "songs": ["song_one.mp3", "song_two.mp3"],
}
```

### load
```json
{
    "kind": "load",
    "playlist": string,
    "range": [integer, integer] (optional),
    "pos": integer (optional),
}
```

Loads the `playlist` to the queue. If `range = [i, j]` is provided, only songs from the `i`-th to the `j`-th one (zero-indexed) are loaded.
If `pos` is provided, then songs are inserted at position `pos` (also zero-indexed), otherwise they're appended to the end.
This command can succeed partially - all songs that were found in the database will be loaded, and the ones that weren't will be returned inside the `reason` key.
A status of `ok` will be returned only if all songs were found.

### removeplaylist
```json
{
    "kind": "removeplaylist",
    "playlist": string,
    "pos": integer,
}
```

Removes the song at position `pos` (zero-indexed) from the `playlist` file.

### save
```json
{
    "kind": "save",
    "path": string,
}
```

Saves the current queue as file at the given `path`. The created file conforms to the M3U format (one song per line).
Song paths are saved as relative to the database's root directory (which makes this operation cross-platform as relative paths are parsed as the same on UNIX and Windows).

## Supported tags
Musing supports the following tags (valid in all requests that require tag names):
- `album`
- `albumartist`
- `arranger`
- `artist`
- `bpm`
- `composer`
- `conductor`
- `date`
- `discnumber`
- `disctotal`
- `ensemble`
- `genre`
- `label`
- `language`
- `lyricist`
- `mood`
- `movementname`
- `movementnumber`
- `part`
- `parttotal`
- `performer`
- `producer`
- `script`
- `sortalbum`
- `sortalbumartist`
- `sortartist`
- `sortcomposer`
- `sorttracktitle`
- `tracknumber`
- `tracktitle`
