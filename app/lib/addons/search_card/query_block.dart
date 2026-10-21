// The `query` block kind: its `/` menu entry, its renderer and its text
// codecs. The block text holds the query string (the core's spec format).

import 'package:flutter/widgets.dart';

import '../../registry/block_codecs.dart';
import '../../registry/block_registry.dart';
import '../../services/addon_services.dart';
import '../../state/app_state.dart';
import '../../ui/icons.dart';
import 'query_card.dart';

/// The core's kind name of a query block.
const searchQueryKind = 'query';

Widget _body(BlockContext b) => QueryCard(
  key: ValueKey('query-block-${b.node.id}'),
  block: b,
  vault: b.context.app.vault,
  // Task rows get a checkbox only while the Tasks add-on is on.
  tasksOn: const AddonServices().addonEnabled('tasks'),
);

/// The core writes a query as a fenced `query` block. The fence is longer
/// than any backtick run in the query, so the query can contain fences.
List<String> _queryFence(String query) {
  var longest = 0;
  for (final run in RegExp(r'`+').allMatches(query)) {
    if (run.group(0)!.length > longest) longest = run.group(0)!.length;
  }
  final fence = '`' * (longest < 3 ? 3 : longest + 1);
  return [
    '${fence}query',
    query.replaceAll('\n', ' '),
    fence,
  ];
}

/// Dart-side codecs. `MarkdownCodec.rust` cannot be used: writing it throws
/// until a synchronous resolver exists. This matches the core's fence.
final searchQueryCodecs = BlockCodecs(
  markdown: MarkdownCodec((n, _) => _queryFence(n.text)),
  clipboard: const ClipboardCodec.viaMarkdown(),
  plainText: PlainTextCodec((n) => n.text),
);

final searchQueryBlock = BlockType(
  kind: searchQueryKind,
  label: 'Search query',
  hint: 'Live results of a search, kept up to date.',
  icon: Ico.search,
  aliases: 'query search results live filter',
  group: 'Advanced',
  editable: false,
  acceptsChildren: false,
  colorable: false,
  insertOnly: true,
  body: _body,
  codecs: searchQueryCodecs,
);
