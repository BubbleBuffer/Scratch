// The kanban's readable Markdown body: one `##` heading per column in stored
// order, then a bullet per card linking it. The page stores it as its
// `$markdown` type-data string (Vault.setTypeMarkdownBody), so exports and
// other readers see the board without the kanban's data shape.
//
// The syntax follows crates/core/src/markdown.rs. A page link is
// `[[target|label]]` (`open_tag`, `close_tag`): the target is the page id, a
// stable identity, and the label is the page's escaped title. A link whose
// label equals its target is written `[[target]]` (`simplify_wikilinks`).
// Text is escaped with the core's character set (`escape`).

/// A column as the readable body shows it.
class KanbanBodyColumn {
  const KanbanBodyColumn(this.title, this.links);

  /// The column's title, written as its heading.
  final String title;

  /// Each card's link, in stored order (see [kanbanCardLink]).
  final List<String> links;
}

/// The characters core escapes in text (`escape` in crates/core/src/markdown.rs).
const _escaped = r'\*_`[]~<>';

/// [text] as one line of escaped markdown text. Line breaks become spaces, and
/// an empty result reads "Untitled".
String kanbanText(String text) {
  final line = text.replaceAll(RegExp(r'\r\n|\r|\n'), ' ').trim();
  if (line.isEmpty) return 'Untitled';
  final out = StringBuffer();
  for (final rune in line.runes) {
    final char = String.fromCharCode(rune);
    if (_escaped.contains(char)) out.write('\\');
    out.write(char);
  }
  return out.toString();
}

/// The link to a card's page: `[[id|title]]`, with the stable page id as the
/// target and the escaped title as the label. The target never depends on the
/// title, so two cards with the same title stay distinct.
String kanbanCardLink(String id, String title) {
  final label = kanbanText(title);
  return label == id ? '[[$id]]' : '[[$id|$label]]';
}

/// The readable body of a board. Each column is a heading, then one bullet
/// per linked card; a column without cards is its heading alone.
String kanbanReadableBody(List<KanbanBodyColumn> columns) => [
  for (final column in columns)
    [
      '## ${kanbanText(column.title)}\n',
      if (column.links.isNotEmpty)
        '\n${[for (final link in column.links) '- $link\n'].join()}',
    ].join(),
].join('\n');
