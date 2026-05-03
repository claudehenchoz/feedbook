# Feed Gallery

## Table of Contents

- [Feed Gallery](#feed-gallery)
  - [Table of Contents](#table-of-contents)
  - [Tech News](#tech-news)
    - [Hacker News](#hacker-news)
    - [Pluralistic](#pluralistic)
    - [Wired](#wired)
    - [Futurism](#futurism)
  - [Swiss News](#swiss-news)
    - [SRF News](#srf-news)
    - [Republik](#republik)
    - [Das Lamm](#das-lamm)
    - [WOZ](#woz)
  - [News](#news)
    - [Wikipedia](#wikipedia)
    - [The Atlantic](#the-atlantic)
    - [The Guardian](#the-guardian)
  - [Gaming](#gaming)
    - [Rock Paper Shotgun](#rock-paper-shotgun)
    - [Jank.cool](#jankcool)


## Tech News

### Hacker News
```
[[feeds]]
url   = "https://hnrss.org/newest?points=100"
name  = "hacker news"
```

### Pluralistic
```
[[feeds]]
url   = "https://pluralistic.net/feed/"
```

### Wired
```
[[feeds]]
url   = "https://www.wired.com/feed/category/big-story/rss"
```

### Futurism
```
[[feeds]]
url   = "https://futurism.com/feed"
```

## Swiss News

### SRF News
```
[[feeds]]
url   = "https://www.srf.ch/news/bnf/rss/1890"
name  = "srf news"
```

### Republik
```
[[feeds]]
url   = "https://www.republik.ch/feed.xml"
```

### Das Lamm
```
[[feeds]]
url   = "https://daslamm.ch/feed/"
content_selectors  = ["div.cs-entry__subtitle","figure.post-media", "div.entry-content"]
remove_selectors   = ["div.wp-block-group"]
name  = "das lamm"
```

### WOZ
```
[[feeds]]
url   = "https://www.woz.ch/t/startseite/feed"
content_selectors  = ["div.article--full"]
remove_selectors   = ["article header", "article aside"]
```


## News

### Wikipedia
```
[[feeds]]
url   = "https://raw.githubusercontent.com/claudehenchoz/wikifeed/refs/heads/main/feed.xml"
name  = "wikipedia"
content_selectors  = ["div#mw-content-text"]
remove_selectors   = ["table.ambox-protection", ".mw-editsection", ".mw-jump-link", ".printfooter", "#siteSub", "#contentSub", "#jump-to-nav", "#catlinks", "#mw-normal-catlinks", "#mw-hidden-catlinks", ".noprint", ".infobox", ".infobox-table", ".sidebar", ".sidebar-collapse", ".ib-legis-elect", ".vertical-navbox", ".navbox", ".navbox-styles", ".navbox-inner", ".navbar", ".hatnote", ".dablink", ".rellink", "#toc", ".toc", "#mw-panel-toc", "#vector-toc", "#vector-page-titlebar-toc", ".mw-collapsible", ".mw-collapsed", ".collapsible", ".reference", ".references", ".reflist", ".mw-references-wrap", ".mw-references-columns", "sup.reference", ".cite-bracket", "style", "script", "link[rel='mw-deduplicated-inline-style']", "noscript", ".mw-editsection-bracket", ".extiw", ".wb-langlinks-edit", "#mw-head", "#mw-panel", "#footer", ".vector-header-container", ".mw-footer", ".vector-page-toolbar", ".vector-sticky-header-container", ".magnify", "h2#References"]
```

### The Atlantic
```
[[feeds]]
url   = "https://www.theatlantic.com/feed/best-of/"
name  = "the atlantic"
```

### The Guardian
```
[[feeds]]
url   = "https://www.theguardian.com/world/rss"
name  = "the guardian"
```

## Gaming

### Rock Paper Shotgun
```
[[feeds]]
url   = "https://www.rockpapershotgun.com/feed"
```

### Jank.cool
```
[[feeds]]
url   = "https://www.jank.cool/rss/"
name  = "jank.cool"
```
