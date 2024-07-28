# nuclino to notion

WIP; do not eat. Might not succeed in migrating to Notion because it doesn't yet handle 409s properly, and ALSO because of missing functionality in the Notion API. Definitely succeeds in making a local backup of Nuclino wiki pages with metadata and media.

This is a command-line tool that migrates a single Nuclino workspace to a set of pages in Notion.

To use it:

1. Make a Nuclino API key. Provide to your environment any way you like as NUCLINO_API_KEY. The tool reads a `.env` file if one is in the directory it's run in.
2. Create a Notion integration. Provide its secret to your environment any way you like as NOTION_API_KEY.
3. In Notion, choose a Nroot page where you want your imported pages to start out. (You can move them later.) In the top right of the Notion window, choose the three-dots menu and connect the root page to your new integration.
4. Use the `share` button to get the link of your chosen root page. The hexadecimal string at the end of the URL is the page id. Make a note of this.
5. Run this tool with an invocation like this: `nuclino-to-notion <hexid>`.


To test in development:

```text
nuc2not cache # fill the cache for a workspace
nuc2not workspace <hexid> # migrate a workspace (unreliably)
nuc2not page <hexid> <hexid> # migrate a single page
```

## Usage

```text
Usage: nuc2not [OPTIONS] <COMMAND>

Commands:
  cache      Cache a Nuclino workspace locally. You'll be prompted to select the workspace
  page       Migrate a single page by id. If the page has media, you'll be prompted to upload
             the media by hand: the Notion API does not have endpoints for doing this
             automatically
  workspace  Migrate a previously-cached Nuclino workspace to Notion. Unreliable!!
  help       Print this message or the help of the given subcommand(s)

Options:
  -w, --wait <WAIT>  How many milliseconds to wait between Nuclino requests [default: 750]
  -h, --help         Print help
  -V, --version      Print version
```

## LICENSE

This code is licensed via [the Parity Public License.](https://paritylicense.com) This license requires people who build on top of this source code to share their work with the community, too. See the license text for details.
