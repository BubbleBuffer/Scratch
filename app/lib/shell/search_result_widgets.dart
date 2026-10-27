// The search result widgets shared by the desktop overlay, the mobile search
// screen and home's inline results: one result row (page or chat), the Pages
// and Chats group headers, the highlighted text and the grouped list.

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../state/app_state.dart';
import '../state/pages_store.dart';
import '../ui/ui.dart';
import 'navigation.dart';
import 'addons.dart';
import 'page_stack.dart';
import 'search_groups.dart';

/// The path of page [id] for search results: its ancestors' titles joined
/// with " / " (empty for a top-level page, or while they load).
String pagePath(PagesStore pages, String id) =>
    pages.ancestors(id).map((m) => displayTitle(m.title, firstLine: m.firstLine)).join(' / ');

/// [text] with each match of [terms] (case-insensitive) in [mark]. Without
/// terms it is a plain [Text], so the text stays as the core returned it.
class HighlightedText extends StatelessWidget {
  const HighlightedText(
    this.text, {
    super.key,
    required this.style,
    this.terms = const [],
    this.maxLines = 1,
  });
  final String text;
  final TextStyle style;
  final List<String> terms;
  final int maxLines;

  @override
  Widget build(BuildContext context) {
    final mark = context.colors.textSelection;
    final pattern = terms.isEmpty
        ? null
        : RegExp(terms.map(RegExp.escape).join('|'), caseSensitive: false);
    if (pattern == null || !pattern.hasMatch(text)) {
      return Text(
        text,
        maxLines: maxLines,
        overflow: TextOverflow.ellipsis,
        style: style,
      );
    }
    final spans = <InlineSpan>[];
    var at = 0;
    for (final m in pattern.allMatches(text)) {
      if (m.start > at) spans.add(TextSpan(text: text.substring(at, m.start)));
      spans.add(
        TextSpan(
          text: text.substring(m.start, m.end),
          style: TextStyle(backgroundColor: mark),
        ),
      );
      at = m.end;
    }
    if (at < text.length) spans.add(TextSpan(text: text.substring(at)));
    return Text.rich(
      TextSpan(style: style, children: spans),
      maxLines: maxLines,
      overflow: TextOverflow.ellipsis,
    );
  }
}

/// One search result. A page shows its icon, title, path and snippet. A chat
/// shows its breadcrumb (the core's display title: chat › chapter › section)
/// and its summary, which is the snippet.
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
  static double extent(bool mobile) => mobile ? 72 : 64;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final m = context.mobile;
    final snippet = hit.snippet.replaceAll('\n', ' ').trim();
    if (isExternalHit(hit)) return _externalRow(context, snippet, c, ty, m);
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
    final titleStyle = ts(
      m ? ty.body : ty.ui,
      empty ? c.textSecondary : c.textPrimary,
      weight: 500,
    );
    return _frame(
      key: ValueKey('hit-${hitRowId(hit)}'),
      icon: PageIcon(
        icon: meta?.icon,
        size: m ? 22 : 20,
        color: c.iconSecondary,
      ),
      mobile: m,
      children: [
        label.firstLine
            ? HighlightedText(
                label.text,
                style: firstLineStyle(titleStyle, c.textTertiary),
                terms: terms,
              )
            : HighlightedText(label.text, style: titleStyle, terms: terms),
        if (path.isNotEmpty)
          Text(
            path,
            key: ValueKey('hit-path-${hit.id}'),
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: ts(ty.uiSmall, c.textTertiary, weight: 400),
          ),
        if (snippet.isNotEmpty && label.text != snippet)
          HighlightedText(
            snippet,
            style: ts(ty.uiSmall, c.textSecondary, weight: 400),
            terms: terms,
          ),
      ],
    );
  }

  Widget _externalRow(
    BuildContext context,
    String snippet,
    AppColors c,
    TypeScale ty,
    bool m,
  ) {
    final source = appAddonContext.searchSources.of(hit.source);
    final title = hit.title.isEmpty
        ? (source?.emptyTitle ?? 'Untitled')
        : hit.title;
    final rowId = hitRowId(hit);
    return _frame(
      key: ValueKey('hit-$rowId'),
      icon: Icon20(
        source?.icon ?? Ico.page,
        size: m ? 22 : 20,
        color: c.iconSecondary,
      ),
      mobile: m,
      children: [
        HighlightedText(
          title,
          maxLines: m ? 2 : 1,
          style: ts(
            m ? ty.body : ty.ui,
            hit.title.isEmpty ? c.textSecondary : c.textPrimary,
            weight: 500,
          ),
          terms: terms,
        ),
        if (snippet.isNotEmpty)
          HighlightedText(
            snippet,
            key: ValueKey('hit-snippet-$rowId'),
            style: ts(ty.uiSmall, c.textSecondary, weight: 400),
            terms: terms,
          ),
      ],
    );
  }

  Widget _frame({
    required Key key,
    required Widget icon,
    required bool mobile,
    required List<Widget> children,
  }) => Hover(
    key: key,
    onTap: onTap,
    radius: mobile ? 0 : Radii.md,
    padding: EdgeInsets.symmetric(horizontal: mobile ? Layout.gutter : 8),
    child: Row(
      children: [
        icon,
        SizedBox(width: mobile ? 12 : 10),
        Expanded(
          child: Column(
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.start,
            children: children,
          ),
        ),
      ],
    ),
  );
}

/// The small group label above a run of results: PAGES or CHATS, in the
/// mono face the mockup uses for group labels.
class SearchGroupHeader extends StatelessWidget {
  const SearchGroupHeader(this.group, {super.key});
  final SearchGroup group;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final m = context.mobile;
    final label = group.label;
    return Padding(
      key: ValueKey('search-group-${group.name}'),
      padding: EdgeInsets.fromLTRB(m ? Layout.gutter : 10, 10, 10, 4),
      child: Text(
        label.toUpperCase(),
        style: ts(ty.uiSmall, c.textTertiary, weight: 500, mono: true),
      ),
    );
  }
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
    final c = context.colors;
    // Headers have no index; rows carry their position in the flat order.
    final items = <({SearchGroup? header, Hit? hit, int index})>[];
    var index = 0;
    for (final section in sections) {
      items.add((header: section.group, hit: null, index: -1));
      for (final hit in section.hits) {
        items.add((header: null, hit: hit, index: index++));
      }
    }
    return ListView.builder(
      padding: padding,
      itemCount: items.length,
      itemBuilder: (context, i) {
        final item = items[i];
        final hit = item.hit;
        if (hit == null) return SearchGroupHeader(item.header!);
        final row = SizedBox(
          height: SearchResultRow.extent(mobile),
          child: SearchResultRow(
            hit: hit,
            onTap: () => onPick(hit),
            terms: terms,
          ),
        );
        if (mobile) return row;
        return DecoratedBox(
          decoration: BoxDecoration(
            color: item.index == active ? c.bgHover : null,
            borderRadius: Radii.ctl,
          ),
          child: row,
        );
      },
    );
  }
}
