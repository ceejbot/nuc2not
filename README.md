# nuclino to notion

WIP; do not eat.

This is a command-line tool that migrates a single Nuclino workspace to a set of pages in Notion.

To use it:

1. Make a Nuclino API key. Provide to your environment any way you like as NUCLINO_API_KEY. The tool reads a `.env` file if one is in the directory it's run in.
2. Create a Notion integration. Provide its secret to your environment any way you like as NOTION_API_KEY.
3. In Notion, choose a Nroot page where you want your imported pages to start out. (You can move them later.) In the top right of the Notion window, choose the three-dots menu and connect the root page to your new integration.
4. Use the `share` button to get the link of your chosen root page. The hexadecimal string at the end of the URL is the page id. Make a note of this.
5. Run this tool with an invocation like this: `nuclino-to-notion <hexid>`.


To test in development:

```text
nuc2not --populate # fill the cache only
nuc2not <hexid> # as many times as you like without exploding your nuclino rate limit
```

## Usage

```text
Usage: nuc2not [OPTIONS] [PARENT]

Arguments:
  [PARENT]  An optional parent page for the imported items. If not provided, the tool won't try migrate pages to Notion

Options:
  -p, --populate     Populate the cache for the chosen Nuclino workspace.
  -w, --wait <WAIT>  How many milliseconds to wait between Nuclino requests [default: 500]
  -h, --help         Print help
  -V, --version      Print version
```

## LICENSE

This code is licensed via [the Parity Public License.](https://paritylicense.com) This license requires people who build on top of this source code to share their work with the community, too. See the license text for details.
