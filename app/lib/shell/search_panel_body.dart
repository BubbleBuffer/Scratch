// The desktop search overlay's content: bb_ui's search panel over the vault's
// hits, with Scratch's type chips and the `type:` rewrite behind them.

import 'dart:async';

import 'package:bb_ui/search.dart' as bb;
import 'package:flutter/widgets.dart';

import '../core/models.dart';
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

  @override
  Widget build(BuildContext context) {
    final hits = widget.hits;
    final shown = hits == null
        ? null
        : searchSectionsOf(context, groupSearchHits(hits), terms: widget.terms);
    final sections = shown?.$1;
    final hitOf = shown?.$2;
    return bb.SearchPanel(
      key: const ValueKey('search-overlay'),
      controller: widget.controller,
      fieldKey: const ValueKey('search-overlay-field'),
      placeholder: widget.mixed ? 'Search pages and chats…' : null,
      onQueryChanged: widget.onQueryChanged,
      onPick: (result) => widget.onPick(hitOf![result]!),
      sections: sections,
      filters: SearchTypeChips(
        active: widget.activeTypes,
        onPick: (types) => unawaited(_setTypes(types)),
      ),
      idleNote: widget.mixed
          ? 'Search page titles, text and chats.'
          : 'Search page titles and text.',
      narrowedNote: widget.narrowed
          ? 'Showing title matches — keep typing to search page text'
          : null,
      correction: widget.correction,
      // A new result list starts at the first row, as the overlay always did.
      selectionResetKey: widget.hits,
    );
  }
}
