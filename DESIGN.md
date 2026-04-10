# feedbook

`feedbook` is a Rust CLI app that creates beautiful EPUB files from an RSS/ATOM feed.

It supports a `url` parameter to an RSS/ATOM feed, and optionally a `limit` parameter to limit the amount of items in the feed.

It will then do this:

1. Retrieve the feed
2. Retrieve the individual articles with the `article_scraper` crate (up to 5 parallel threads for speed)
3. Generate an EPUB file that contains all the articles, with `title`, `author`, `url` and `date` as the header of the article, and the `html` as the rest of the article

## Details

* Keep a local cache in a SQLite database that hosts the retrieved articles, so that future hits against the same XML feed only fetch the new items (and the already-downloaded ones are retrieved from the cache)
* Store the SQLite database somewhere in the user profile as feedbook.sql
* Implement a sane SQLite database pruning method so that it doesn't grow into crazy sizes
* Focus on absolute speed, it should feel instant to generate an EPUB file
