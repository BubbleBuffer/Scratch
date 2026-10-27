// The desktop search overlay's content: the field, the type chips, the grouped
// results and the explanation lines. The body owns its keyboard selection.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../ui/ui.dart';
import 'search_groups.dart';
import 'search_result_widgets.dart';
import 'search_type_chips.dart';

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
