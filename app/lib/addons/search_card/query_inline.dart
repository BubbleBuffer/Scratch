// Form B's inline chip, as the search card provides it: a query mark in a
// paragraph draws the live count of its query, and a tap opens the results
// panel. The editor owns the chip's state (see RichController.syncInline); this
// file makes the state, reads its label, opens the panel and converts the
// paragraph to or from a query block through the editor's BlockHost.

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../registry/addon_registry.dart';
import '../../services/addon_services.dart';
import '../../services/legacy_host.dart' show showSearchOverlay;
import 'query_block.dart';
import 'query_panel.dart';
import 'query_watch.dart';

class QueryInlineMark implements AddonInlineMark {
  const QueryInlineMark();

  @override
  String get id => 'search_card.query';

  @override
  BMarkKind get kind => BMarkKind.query;

  @override
  ChangeNotifier watch(BuildContext context, String payload) => QueryWatch(
    vault: const AddonServices().vault(context),
    tasksOn: const AddonServices().addonEnabled('tasks'),
    query: payload,
  )..start();

  /// The count's chip text, once the first result is in.
  @override
  String? label(ChangeNotifier state) {
    final watch = state as QueryWatch;
    return watch.ready ? watch.wording.chipLabel : null;
  }

  @override
  void open(BuildContext context, Rect anchor, InlineMarkRef ref) {
    showQueryPanel(
      context,
      watch: ref.state as QueryWatch,
      anchor: anchor,
      onOpenInSearch: () => showSearchOverlay(context, query: ref.payload),
      onShowAsBlock: () => showQueryAsBlock(ref),
    );
  }
}

/// "Show as a block": when the chip is the paragraph's only content, the
/// paragraph becomes the query block. Otherwise the block goes after the
/// paragraph and the chip's text is cut from it, with one adjacent space (the
/// one after it, else the one before it) so the words keep one space between
/// them. Either is one undo step.
void showQueryAsBlock(InlineMarkRef ref) {
  final block = ref.block;
  final data = {'text': ref.payload};
  final onlyChip =
      ref.range.start == 0 &&
      ref.range.end == block.text.length &&
      block.children.isEmpty;
  if (onlyChip) {
    ref.host.replaceWithBlock(block, searchQueryKind, data);
  } else {
    ref.host.insertBlockAfter(
      block,
      searchQueryKind,
      data,
      removeText: _chipCut(block.text, ref.range),
    );
  }
}

/// [range] widened by one space: the space after it if there is one, else the
/// space before it. Other whitespace is left as it is.
TextRange _chipCut(String text, TextRange range) {
  if (range.end < text.length && text[range.end] == ' ') {
    return TextRange(start: range.start, end: range.end + 1);
  }
  if (range.start > 0 && text[range.start - 1] == ' ') {
    return TextRange(start: range.start - 1, end: range.end);
  }
  return range;
}
