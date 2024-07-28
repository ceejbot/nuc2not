# Nuc2Not

[![Tests](https://github.com/ceejbot/nuc2not/actions/workflows/test.yaml/badge.svg)](https://github.com/ceejbot/nuc2not/actions/workflows/test.yaml)

This is a command-line tool that migrates Nuclino wiki pages to Notion pages.

The tool does not fully migrate content to Notion because of missing functionality in the Notion API. Specifically, you can't upload images or file downloads via its public API. This tool definitely succeeds in making a local backup of Nuclino wiki pages with metadata and media, but it's reduced to prompting you to upload media by hand.

You'll need to do some steps in both Nuclino and Notion to set yourself up to use their APIs.

1. Make a Nuclino API key. Provide this key to your environment any way you like as `NUCLINO_API_KEY`. `nuc2not` reads a `.env` file if one is in the directory it's run in.
2. Create a Notion integration. Provide its secret to your environment any way you like as `NOTION_API_KEY`.
3. In Notion, choose a root page where you want your imported pages to start out. (You can move them later.) In the top right of the Notion window, choose the three-dots menu and connect the root page to your new integration.
4. Use the `share` button to get the link of your chosen root page. The hexadecimal string at the end of the URL is the page id. Make a note of this; you'll need to provide it to the tool for all migration actions.
5. Run this tool to create a cache of the workspace or workspaces you want to migrate.
6. Then migrate some or all of the pages.

You must cache pages before trying to migrate them. This is a design choice. My goal was to back up our Nuclino wiki and all its meta data just to have it around, then decide what to do with it.

```text
nuc2not cache # fill the cache for a workspace
nuc2not migrate-workspace <notion-parent-id> # migrate a entire cached workspace
nuc2not migrate-page -p <parent-id> <page-id> <page-id> # migrate a few pages
```

## Usage

Each subcommand has more detailed help.

```text
Usage: nuc2not [OPTIONS] <COMMAND>

Commands:
  cache              Cache a Nuclino workspace locally. You'll be prompted to select the workspace
  inspect-cache      Inspect your local cache, listing pages by id
  migrate-page       Migrate a single page by id. If the page has media, you'll be prompted to
                     upload the media by hand: the Notion API does not have endpoints for doing
                     this automatically
  migrate-workspace  Migrate a previously-cached Nuclino workspace to Notion. Unreliable!!
  help               Print this message or the help of the given subcommand(s)

Options:
  -w, --wait <WAIT>  How many milliseconds to wait between Nuclino requests [default: 750]
  -h, --help         Print help
  -V, --version      Print version
  ```

## TODO list

Here are some features I'm contemplating implementing.

- [ ] Updating migrated Nuclino pages with links to their Notion versions. Should be easy.
- [ ] Doing something to connect migrated pages with author information, even if I can't set a page's author directly when creating a page. Less easy than the link-back, because it involves constructing Notion block content, but still not hard.
- [ ] Doing something smarter than just blanket waits between requests to avoid hitting rate limits. In particular, a page with lots of deep nested lists that force lots of repeated append calls can take a long time to create, even when it's not a lot of content in word count.

## LICENSE

This code is licensed via [the Parity Public License.](https://paritylicense.com) This license requires people who build on top of this source code to share their work with the community, too. See the license text for details.
