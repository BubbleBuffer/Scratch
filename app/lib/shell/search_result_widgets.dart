// The search result widgets of the desktop overlay, the mobile search screen
// and home's inline results. The drawing is bb_ui's (package:bb_ui/search.dart);
// this file maps a vault or add-on [Hit] onto its neutral view model (title
// tone, icon, path, highlights) and keeps the Hit-typed API the screens use.

import 'package:bb_ui/search.dart' as bb;
import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../state/app_state.dart';
import '../state/pages_store.dart';
import '../ui/ui.dart';
import 'addons.dart';
import 'page_stack.dart';
import 'search_groups.dart';

/// The path of page [id] for search results: its ancestors' titles joined
/// with " / " (empty for a top-level page, or while they load).
String pagePath(PagesStore pages, String id) => pages
    .ancestors(id)
    .map((m) => displayTitle(m.title, firstLine: m.firstLine))
    .join(' / ');

/// The view model of [hit]: a page shows its icon, title, path and snippet,
/// an add-on hit (a chat) its source's icon, its breadcrumb (the core's
/// display title) and its summary, which is the snippet. [terms] are the words
/// to highlight in the title and snippet.
bb.SearchResult searchResultOf(
  BuildContext context,
  Hit hit, {
  List<String> terms = const [],
}) {
  final c = context.colors;
  final m = context.mobile;
  final snippet = hit.snippet.replaceAll('\n', ' ').trim();
  final rowId = hitRowId(hit);
  if (isExternalHit(hit)) {
    final source = appAddonContext.searchSources.of(hit.source);
    final title = hit.title.isEmpty
        ? (source?.emptyTitle ?? 'Untitled')
        : hit.title;
    return bb.SearchResult(
      id: rowId,
      title: title,
      titleTone: hit.title.isEmpty
          ? bb.SearchTitleTone.muted
          : bb.SearchTitleTone.normal,
      titleMaxLines: m ? 2 : 1,
      titleHighlights: bb.termHighlights(title, terms),
      snippet: snippet,
      snippetKey: ValueKey('hit-snippet-$rowId'),
      snippetHighlights: bb.termHighlights(snippet, terms),
      icon: source?.icon ?? Ico.page,
      rowKey: ValueKey('hit-$rowId'),
    );
  }
  final pages = context.watch<PagesStore>();
  final meta = pages.meta(hit.id);
  final path = pagePath(pages, hit.id);
  final empty = hit.title.isEmpty;
  // An untitled page goes by its first line, else by the matching text the
  // index returns.
  var label = pageLabelOf(context, hit.id, hit.title);
  if (empty && !label.firstLine && snippet.isNotEmpty) {
    label = (text: snippet, firstLine: true);
  }
  final shownSnippet = snippet.isNotEmpty && label.text != snippet
      ? snippet
      : '';
  return bb.SearchResult(
    id: rowId,
    title: label.text,
    titleTone: label.firstLine
        ? bb.SearchTitleTone.standIn
        : empty
        ? bb.SearchTitleTone.muted
        : bb.SearchTitleTone.normal,
    titleHighlights: bb.termHighlights(label.text, terms),
    secondary: path,
    secondaryKey: ValueKey('hit-path-${hit.id}'),
    snippet: shownSnippet,
    snippetHighlights: bb.termHighlights(shownSnippet, terms),
    leading: PageIcon(
      icon: meta?.icon,
      size: m ? 22 : 20,
      color: c.iconSecondary,
    ),
    rowKey: ValueKey('hit-$rowId'),
  );
}

/// One search result for a [hit], drawn by bb_ui's row.
class SearchResultRow extends StatelessWidget {
  const SearchResultRow({
    super.key,
    required this.hit,
    required this.onTap,
    this.terms = const [],
  });
  final Hit hit;
  final VoidCallback onTap;

  /// The words to highlight in the title and snippet.
  final List<String> terms;

  /// The row's height.
  static double extent(bool mobile) => bb.SearchResultRow.extent(mobile);

  @override
  Widget build(BuildContext context) => bb.SearchResultRow(
    result: searchResultOf(context, hit, terms: terms),
    onTap: onTap,
  );
}

/// The grouped results: a header per group, then its rows. [active] is the
/// keyboard selection, an index into the flattened hits (desktop only).
class SearchResultsList extends StatelessWidget {
  const SearchResultsList({
    super.key,
    required this.sections,
    required this.onPick,
    required this.mobile,
    this.active,
    this.terms = const [],
    this.padding = EdgeInsets.zero,
  });
  final List<SearchSection> sections;
  final ValueChanged<Hit> onPick;
  final bool mobile;
  final int? active;
  final List<String> terms;
  final EdgeInsets padding;

  @override
  Widget build(BuildContext context) {
    final (shared, hitOf) = searchSectionsOf(context, sections, terms: terms);
    return bb.SearchResultsList(
      sections: shared,
      mobile: mobile,
      active: active,
      padding: padding,
      onPick: (result) => onPick(hitOf[result]!),
    );
  }
}

/// [sections] as view models, and the [Hit] each result stands for.
(List<bb.SearchResultSection>, Map<bb.SearchResult, Hit>) searchSectionsOf(
  BuildContext context,
  List<SearchSection> sections, {
  List<String> terms = const [],
}) {
  final hitOf = Map<bb.SearchResult, Hit>.identity();
  final shared = [
    for (final section in sections)
      bb.SearchResultSection(
        id: section.group.name,
        label: section.label,
        results: [
          for (final hit in section.hits)
            () {
              final result = searchResultOf(context, hit, terms: terms);
              hitOf[result] = hit;
              return result;
            }(),
        ],
      ),
  ];
  return (shared, hitOf);
}
