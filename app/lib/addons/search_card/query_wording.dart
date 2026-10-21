// The count wording and the empty line of a query, from its parsed tokens
// (the core's parse_search). Pure Dart: no widgets, no bridge calls, so the
// rules are unit-tested. Form B's chip reuses [queryWording].
//
// Patterns follow the approved mockup and apply only to what the query says:
// a task list (type:task or a done: filter, with tags), a status column
// (status:<value>), or pages edited this week (updated:week). Anything else
// gets the generic "results" wording. Negated operators and sort/in do not
// shape the wording, except `-status:done` on a task list, which reads "open".

import 'dart:convert';

import '../../src/rust/api/types.dart';

/// Wording for one count of one query.
class QueryWording {
  const QueryWording({required this.chipLabel, required this.emptyLine});

  /// The chip text for the count, e.g. "4 open #work tasks" or
  /// "1 page this week". Zero counts read "no …" for task lists.
  final String chipLabel;

  /// The line shown when there are no rows, e.g. "No open #work tasks.".
  final String emptyLine;
}

/// Keys are case-insensitive and ignore spaces, `_` and `-`, as the core
/// normalizes them (`duedate:` matches "Due date").
String normalizeQueryKey(String key) =>
    key.replaceAll(RegExp(r'[\s_-]'), '').toLowerCase();

QueryWording queryWording({
  required String query,
  required List<Token> tokens,
  required int count,
}) {
  final ops = _positiveOperators(query, tokens);
  bool has(String key, bool Function(String value) test) =>
      ops.any((o) => o.key == key && test(o.value));

  final week = has('updated', (v) => v.toLowerCase() == 'week');
  final filters = ops.where((o) => o.key != 'sort' && o.key != 'in');
  if (week && filters.every((o) => o.key == 'updated')) {
    return QueryWording(
      chipLabel: count == 1 ? '1 page this week' : '$count pages this week',
      emptyLine: 'Nothing edited this week.',
    );
  }

  final doneOps = ops.where((o) => o.key == 'done');
  final taskType = has('type', (v) => v.toLowerCase() == 'task');
  if (taskType || doneOps.isNotEmpty) {
    // `-status:done` is how a task list asks for open tasks.
    final notDone = _operators(query, tokens, negated: true).any(
      (o) => o.key == 'status' && o.value.toLowerCase() == 'done',
    );
    final open =
        notDone ||
        doneOps.any((o) => const {'no', 'false'}.contains(o.value.toLowerCase()));
    final tags = [for (final o in ops) if (o.key == 'tag') '#${o.value}'];
    final phrase = [
      if (open) 'open',
      ...tags,
      count == 1 ? 'task' : 'tasks',
    ].join(' ');
    return QueryWording(
      chipLabel: count == 0 ? 'no $phrase' : '$count $phrase',
      emptyLine: '${_capitalize('no $phrase')}.',
    );
  }

  final status = ops.where((o) => o.key == 'status').firstOrNull;
  if (status != null) {
    final column = _capitalize(status.value);
    return QueryWording(
      chipLabel: '$count in $column',
      emptyLine: 'Nothing in $column.',
    );
  }

  final text = query.trim();
  return QueryWording(
    chipLabel: count == 1 ? '1 result · $text' : '$count results · $text',
    emptyLine: 'No results for $text.',
  );
}

class _Operator {
  const _Operator(this.key, this.value);
  final String key;
  final String value;
}

/// The operators the query states without a minus sign.
List<_Operator> _positiveOperators(String query, List<Token> tokens) =>
    _operators(query, tokens, negated: false);

/// The operators with (`negated: true`) or without a minus sign. The token
/// span starts at its first byte, so a `-` there marks a negation. Spans are
/// UTF-8 offsets.
List<_Operator> _operators(
  String query,
  List<Token> tokens, {
  required bool negated,
}) {
  final bytes = utf8.encode(query);
  return [
    for (final token in tokens)
      if (token.kind.kind == SearchTokenKind.operator_ &&
          (token.start < bytes.length && bytes[token.start] == 0x2d) == negated)
        _Operator(_typedKey(bytes, token), token.kind.value),
  ];
}

/// The key as the user typed it, read from the token's own span. The core
/// resolves a property key (`status:`) to the property's id, so the typed
/// word is the one that names it. Tags (`#x`) are the `tag` key.
String _typedKey(List<int> bytes, Token token) {
  final end = token.end.clamp(token.start, bytes.length);
  var word = utf8.decode(bytes.sublist(token.start, end));
  if (word.startsWith('-')) word = word.substring(1);
  if (word.startsWith('#')) return 'tag';
  final colon = word.indexOf(':');
  return normalizeQueryKey(colon > 0 ? word.substring(0, colon) : token.kind.key);
}

String _capitalize(String text) =>
    text.isEmpty ? text : text[0].toUpperCase() + text.substring(1);
