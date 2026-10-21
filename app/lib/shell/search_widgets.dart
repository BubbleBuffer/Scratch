// The search widgets shared by the desktop overlay, the mobile search screen
// and home's inline results: one result row (page or chat), the Pages and
// Chats group headers, the type chips that write `type:` into the query, and
// the grouped list. The overlay body owns its keyboard selection.

import 'dart:async';

import 'package:flutter/services.dart';
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

/// The label of a dynamic type chip: the plural of a known type, else the
/// value as typed.
String typeChipLabel(String value) => switch (value) {
  'document' => 'Documents',
  'database' => 'Databases',
  _ =>
    appAddonContext.searchSources.all
            .where((source) => source.noteTypes.contains(value))
            .firstOrNull
            ?.label ??
        value,
};

/// The type filter chips. All, Pages and Chats are fixed: each is selected
/// when the query's type values are exactly its set. A query whose type
/// values match no fixed set gets one selected, removable chip per value
/// (`type:document` shows "Documents"). [onPick] receives the type values to
/// write (empty for All).
class SearchTypeChips extends StatelessWidget {
  const SearchTypeChips({
    super.key,
    required this.active,
    required this.onPick,
  });
  final Set<String> active;
  final ValueChanged<Set<String>> onPick;

  @override
  Widget build(BuildContext context) {
    final fixed = <(String, String, Set<String>)>[
      ('all', 'All', const {}),
      ('pages', 'Pages', searchTypePages),
      for (final source in appAddonContext.searchSources.all)
        if (source.noteTypes.isNotEmpty)
          (source.groupId, source.label, source.noteTypes),
    ];
    final matchesFixed = fixed.any((f) => _sameTypes(active, f.$3));
    final dynamicValues = matchesFixed
        ? const <String>[]
        : (active.toList()..sort());
    return Wrap(
      spacing: 6,
      runSpacing: 6,
      children: [
        for (final (id, label, types) in fixed)
          _TypeChip(
            key: ValueKey('search-type-$id'),
            label: label,
            selected: types.isEmpty
                ? active.isEmpty
                : _sameTypes(active, types),
            onTap: () {
              final selected = types.isEmpty
                  ? active.isEmpty
                  : _sameTypes(active, types);
              // Tapping the selected chip returns to All.
              onPick(selected ? const {} : types);
            },
          ),
        for (final value in dynamicValues)
          _TypeChip(
            key: ValueKey('search-type-value-$value'),
            label: typeChipLabel(value),
            selected: true,
            removable: true,
            // Removing a dynamic chip drops only its own value.
            onTap: () => onPick({
              for (final v in active)
                if (v != value) v,
            }),
          ),
      ],
    );
  }
}

bool _sameTypes(Set<String> a, Set<String> b) =>
    a.length == b.length && a.containsAll(b);

class _TypeChip extends StatelessWidget {
  const _TypeChip({
    super.key,
    required this.label,
    required this.selected,
    required this.onTap,
    this.removable = false,
  });
  final String label;
  final bool selected;
  final VoidCallback onTap;

  /// Whether the chip ends with a close mark (its tap removes the value).
  final bool removable;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final m = context.mobile;
    final ink = selected ? c.bgPrimary : c.textSecondary;
    return Semantics(
      button: true,
      selected: selected,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        // No alignment here: an aligned Container fills the Wrap's width, and
        // each chip would then take its own row.
        child: Container(
          constraints: BoxConstraints(minHeight: m ? Layout.minTapTarget : 24),
          padding: EdgeInsets.symmetric(horizontal: m ? 12 : 10),
          decoration: BoxDecoration(
            color: selected ? c.textPrimary : null,
            border: Border.all(color: selected ? c.textPrimary : c.border),
            borderRadius: Radii.pill,
          ),
          // Centred at the content's own width, so the pill keeps its minimum
          // height without stretching across the Wrap.
          child: Center(
            widthFactor: 1,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  label,
                  style: ts(m ? ty.ui : ty.uiSmall, ink, weight: 500),
                ),
                if (removable) ...[
                  const SizedBox(width: 4),
                  Icon20(Ico.close, size: 12, color: ink),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The desktop overlay's content: the field, the type chips, the grouped
/// results and the explanation lines. Up and down move the selection through
/// the flattened results, Enter picks it, and the field takes focus on open.
class SearchPanelBody extends StatefulWidget {
  const SearchPanelBody({
    super.key,
    required this.controller,
    required this.onQueryChanged,
    required this.onPick,
    required this.hits,
    required this.rewriteTypes,
    this.narrowed = false,
    this.correction,
    this.terms = const [],
    this.activeTypes = const {},
    this.mixed = false,
  });
  final TextEditingController controller;

  /// Whether the hits come from pages and chats (the global search) rather
  /// than pages alone: the field's hint and the empty note name both.
  final bool mixed;

  /// Called with the new query text after the user types or taps a chip.
  final ValueChanged<String> onQueryChanged;
  final ValueChanged<Hit> onPick;

  /// The query text with its `type:` operators replaced by [types] (empty
  /// removes them), or null when it cannot be rewritten. The owner parses the
  /// text, so this stays free of the parser.
  final Future<String?> Function(String text, Set<String> types) rewriteTypes;

  /// The results for the current query, or null when it is empty.
  final List<Hit>? hits;
  final bool narrowed;
  final String? correction;
  final List<String> terms;
  final Set<String> activeTypes;

  @override
  State<SearchPanelBody> createState() => _SearchPanelBodyState();
}

class _SearchPanelBodyState extends State<SearchPanelBody> {
  late final _focus = FocusNode(onKeyEvent: _key);
  int _active = 0;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _focus.requestFocus();
    });
  }

  @override
  void didUpdateWidget(SearchPanelBody old) {
    super.didUpdateWidget(old);
    // A new result list starts at the first row, as the overlay always did.
    if (!identical(old.hits, widget.hits)) _active = 0;
  }

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  List<Hit> get _order =>
      flattenSections(groupSearchHits(widget.hits ?? const []));

  Future<void> _setTypes(Set<String> types) async {
    final text = widget.controller.text;
    final next = await widget.rewriteTypes(text, types);
    // The field changed while the parse ran: the newer text wins.
    if (!mounted || next == null || widget.controller.text != text) return;
    widget.controller.value = TextEditingValue(
      text: next,
      selection: TextSelection.collapsed(offset: next.length),
    );
    widget.onQueryChanged(next);
  }

  KeyEventResult _key(FocusNode _, KeyEvent e) {
    if (e is! KeyDownEvent && e is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    final hits = _order;
    final k = e.logicalKey;
    if (k == LogicalKeyboardKey.arrowDown && hits.isNotEmpty) {
      setState(() => _active = (_active + 1) % hits.length);
      return KeyEventResult.handled;
    }
    if (k == LogicalKeyboardKey.arrowUp && hits.isNotEmpty) {
      setState(() => _active = (_active - 1 + hits.length) % hits.length);
      return KeyEventResult.handled;
    }
    if ((k == LogicalKeyboardKey.enter ||
            k == LogicalKeyboardKey.numpadEnter) &&
        hits.isNotEmpty) {
      widget.onPick(hits[_active.clamp(0, hits.length - 1)]);
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final hits = widget.hits;
    final correction = widget.correction;
    final sections = groupSearchHits(hits ?? const []);
    Widget note(String s) => Padding(
      padding: const EdgeInsets.all(16),
      child: Text(s, style: ts(ty.ui, c.textTertiary, weight: 400)),
    );
    return Column(
      key: const ValueKey('search-overlay'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 12, 12, 8),
          child: SearchField(
            fieldKey: const ValueKey('search-overlay-field'),
            controller: widget.controller,
            focusNode: _focus,
            onChanged: widget.onQueryChanged,
            placeholder: widget.mixed ? 'Search pages and chats…' : null,
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(12, 0, 12, 8),
          child: SearchTypeChips(
            active: widget.activeTypes,
            onPick: (types) => unawaited(_setTypes(types)),
          ),
        ),
        HLine(color: c.divider),
        Expanded(
          child: hits == null
              ? note(
                  widget.mixed
                      ? 'Search page titles, text and chats.'
                      : 'Search page titles and text.',
                )
              : hits.isEmpty
              ? note('No results')
              : SearchResultsList(
                  sections: sections,
                  mobile: false,
                  active: _active.clamp(0, hits.length - 1),
                  terms: widget.terms,
                  padding: const EdgeInsets.all(6),
                  onPick: widget.onPick,
                ),
        ),
        if (widget.narrowed)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 12),
            child: Text(
              'Showing title matches — keep typing to search page text',
              key: const ValueKey('search-narrowed-hint'),
              style: ts(ty.uiSmall, c.textTertiary, weight: 400),
            ),
          ),
        if (correction != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 12),
            child: Text.rich(
              TextSpan(
                children: [
                  const TextSpan(text: 'Showing results for '),
                  TextSpan(
                    text: correction,
                    style: const TextStyle(fontStyle: FontStyle.italic),
                  ),
                ],
              ),
              key: const ValueKey('search-correction-hint'),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(ty.uiSmall, c.textTertiary, weight: 400),
            ),
          ),
      ],
    );
  }
}
