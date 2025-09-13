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

Musing responds to every request with a response, which always contains a `status` key with a value of either `ok` or `err`. If `status` is `err`, then there will be a `reason` key present with a string value which describes why the request failed. Beyond that, responses may contain more keys specyfing details related to the given request. All responses are described in detail in the next section.

## Available requests

### ls
Request:
```json
{
    "kind": "ls",
    "dir": <string>,
}
```

Returns paths of all songs located in `dir`. Only songs contained in the Musing database are taken into account.
Should be used only with untagged/badly tagged music collections. With properly tagged collections using `select` will be more convenient.

Response:
```json
{
    "paths": <array of strings>,
}
```

### metadata
Request:
```json
{
    "kind": "metadata",
    "paths": <array of strings>,
    "tags": <array of strings>,
}
```
or
```json
{
    "kind": "metadata",
    "paths": <array of strings>,
    "all_tags": <bool>,
}
```

Returns objects containing key-value pairs representing the metadata of songs located in `paths`. If `tags` is specified, returns only the values corresponding to the provided tags. If `all_tags` is specified and true, returns values of all tags supported by Musing (you can find a list of supported tags at the end of these docs).

Response:
```json
{
    "metadata": <array of objects>,
}
```

### select
Request:
```json
{
    "kind": "select",
    "tags": <array of strings>,
    "filters": <array of objects>,
    "group_by": <array of strings>,
    "comparators": <array of objects>,
}
```

Returns paths and values of `tags` of those songs, which satisfy each of the `filters`. The results are grouped by the values of tags in `group_by` and sorted by `comparators`.

A filter is a JSON object with the following structure:
```json
{
    "kind": "regex",
    "tag": <string>,
    "regex": <string>,
}
```
As the name suggests, it allows only songs whose value of `tag` matches the regular expression `regex` to "pass through". If a song has no defined value for `tag`, it doesn't pass the filter. Regexes are parsed by the `regex` crate, so a reference of their syntax is available [here](https://docs.rs/regex/latest/regex/#syntax).

A comparator in a JSON object with the following structure:
```json
{
    "tag": <string>,
    "order": <string>,
}
```
Its job is to sort the response values according to the value of `tag`. The order is determined by the `order` key, whose only valid values are `"ascending"` and `"descending"`.

Response:
```json
{
    "values": <array of objects>,
}
```

## Supported tags
