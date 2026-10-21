// Pure tests for the search groups: Pages before Chats with the core's order
// kept, chat targets read from JSON (never a title), the `type:` operator the
// chips write (cut by its parsed byte span), and the highlight words.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/search_groups.dart';

import 'search_hits_fixture.dart';

/// One parsed piece of a test query: its token and, for a built-in operator,
/// the spec operator the parser pairs with it.
class _Piece {
  const _Piece(this.token, [this.op]);
  final Token token;
  final SearchOperator? op;
}

/// The UTF-8 byte span of the first occurrence of [text] in [query].
(int, int) _span(String query, String text) {
  final at = query.indexOf(text);
  assert(at >= 0, '$text is not in $query');
  final start = utf8.encode(query.substring(0, at)).length;
  return (start, start + utf8.encode(text).length);
}

/// A phrase, text or property token for [text] in [query].
_Piece _term(
  String query,
  String text, {
  SearchTokenKind kind = SearchTokenKind.phrase,
  String value = '',
}) {
  final (start, end) = _span(query, text);
  return _Piece(
    Token(
      start: start,
      end: end,
      kind: TokenKind(
        kind: kind,
        value: value,
        phrase: kind == SearchTokenKind.phrase,
        key: '',
        message: '',
      ),
    ),
  );
}

/// A built-in operator for [text] in [query] with the spec flags the parser
/// gives it: [key] is the normalized key and [negated] the leading `-`.
_Piece _operator(
  String query,
  String text, {
  required String key,
  required String value,
  bool negated = false,
}) {
  final (start, end) = _span(query, text);
  return _Piece(
    Token(
      start: start,
      end: end,
      kind: TokenKind(
        kind: SearchTokenKind.operator_,
        value: value,
        phrase: false,
        key: key,
        message: '',
      ),
    ),
    SearchOperator(
      key: key,
      value: value,
      negated: negated,
      property: false,
      dateFuture: false,
    ),
  );
}

/// The parse of [query] that the pieces make.
ParsedQuery _parsed(String query, List<_Piece> pieces) => ParsedQuery(
  spec: SearchSpec(
    terms: const [],
    operators: [
      for (final p in pieces)
        if (p.op != null) p.op!,
    ],
    warnings: const [],
  ),
  tokens: [for (final p in pieces) p.token],
);

/// The positive type filters of [query] made of [pieces].
List<TypeFilter> _filters(String query, List<_Piece> pieces) =>
    typeFiltersOf(query, _parsed(query, pieces));

void main() {
  test('groups pages before chats and keeps each group in rank order', () {
    final hits = [
      chatHit('c-1'),
      pageHit('p-1', 'Garden'),
      chatHit('c-2'),
      pageHit('p-2', 'Gardening'),
    ];
    final sections = groupSearchHits(hits);
    expect(sections.map((s) => s.label), ['Pages', 'Chats']);
    expect(sections[0].hits.map((h) => h.id), ['p-1', 'p-2']);
    expect(sections[1].hits.map((h) => h.id), ['c-1', 'c-2']);
    expect(flattenSections(sections).map((h) => h.id), [
      'p-1',
      'p-2',
      'c-1',
      'c-2',
    ]);
  });

  test('an empty group is left out', () {
    expect(groupSearchHits([pageHit('p-1', 'A')]).map((s) => s.label), [
      'Pages',
    ]);
    expect(groupSearchHits([chatHit('c-1')]).map((s) => s.label), ['Chats']);
    expect(groupSearchHits(const []), isEmpty);
  });

  test('row ids keep vault rows bare and prefix other sources', () {
    expect(hitRowId(pageHit('c-1', 'Garden')), 'c-1');
    expect(hitRowId(chatHit('c-1')), 'agent/c-1');
    // A page and a chat that share an id keep separate row ids.
    expect(hitRowId(pageHit('c-1', 'Garden')), isNot(hitRowId(chatHit('c-1'))));
  });

  test('a chat target is read from its JSON, with the anchor range', () {
    final target = ChatSearchTarget.parse(chatHit('c-1').target)!;
    expect(target.sessionId, 'sess-1');
    expect(target.firstMessageId, 'm-53');
    expect(target.lastMessageId, 'm-88');
    expect(target.anchored, isTrue);
    expect(target.refLevel, ChatRefLevel.section);
    expect(target.sectionTitle, 'Chrome live drops pass');
    final chapter = ChatSearchTarget.parse(
      chatHit('c-2', level: 'chapter').target,
    )!;
    expect(chapter.refLevel, ChatRefLevel.chapter);
    expect(chapter.sectionTitle, 'Chrome live drops pass');
  });

  test('a title-only chat target has no range and opens its session', () {
    final target = ChatSearchTarget.parse(
      chatHit('chat', level: 'chat', firstMessage: '', lastMessage: '').target,
    )!;
    expect(target.sessionId, 'sess-1');
    expect(target.anchored, isFalse);
  });

  test('a missing or malformed chat target opens nothing', () {
    expect(ChatSearchTarget.parse(null), isNull);
    expect(ChatSearchTarget.parse(''), isNull);
    expect(ChatSearchTarget.parse('not json'), isNull);
    expect(ChatSearchTarget.parse('{"sessionId":"s"}'), isNull);
    expect(ChatSearchTarget.parse('[1,2]'), isNull);
    expect(
      ChatSearchTarget.parse(
        jsonEncode({
          'sessionId': '',
          'firstMessageId': '',
          'lastMessageId': '',
          'level': 'chat',
        }),
      ),
      isNull,
    );
  });

  test('a positive type operator is a filter at its byte span', () {
    const q = 'garden type:document,database';
    final filters = _filters(q, [
      _operator(
        q,
        'type:document,database',
        key: 'type',
        value: 'document,database',
      ),
    ]);
    expect(filters, hasLength(1));
    expect(filters.single.start, 7);
    expect(filters.single.end, q.length);
    expect(typeValuesOf(filters), {'document', 'database'});
  });

  test('a normalized type key is the built-in type operator', () {
    // The parser ignores `_` and `-` in a built-in key, so these are `type`.
    const q = 'Ty_pe:chat';
    final filters = _filters(q, [
      _operator(q, 'Ty_pe:chat', key: 'type', value: 'chat'),
    ]);
    expect(typeValuesOf(filters), {'chat'});
  });

  test('a negated type operator is not a filter', () {
    const q = '-type:chat garden';
    expect(
      _filters(q, [
        _operator(q, '-type:chat', key: 'type', value: 'chat', negated: true),
      ]),
      isEmpty,
    );
  });

  test('a quoted type key is not a filter', () {
    // The parser reads `"type":chat` as text, so it has no operator at all.
    const q = '"type":chat';
    expect(_filters(q, [_term(q, '"type":chat', value: 'type:chat')]), isEmpty);
  });

  test('another key is never a type value', () {
    const q = 'tag:chat';
    expect(
      _filters(q, [_operator(q, 'tag:chat', key: 'tag', value: 'chat')]),
      isEmpty,
    );
  });

  test('a phrase that holds type: is never a filter', () {
    const q = '"say type:chat"';
    expect(_filters(q, [_term(q, q, value: 'say type:chat')]), isEmpty);
  });

  test('the chips write type: after the other words', () {
    expect(withTypeFilters('', const [], searchTypeChats), 'type:chat');
    expect(
      withTypeFilters('garden tag:work', const [], searchTypePages),
      'garden tag:work type:document,database',
    );
  });

  test('picking another type replaces the old one, other operators stay', () {
    const q = 'type:chat garden';
    final filters = _filters(q, [
      _operator(q, 'type:chat', key: 'type', value: 'chat'),
    ]);
    expect(
      withTypeFilters(q, filters, searchTypePages),
      'garden type:document,database',
    );
    const q2 = 'garden tag:work type:chat';
    final filters2 = _filters(q2, [
      _operator(q2, 'type:chat', key: 'type', value: 'chat'),
    ]);
    expect(withTypeFilters(q2, filters2, {}), 'garden tag:work');
    const q3 = 'type:chat';
    final filters3 = _filters(q3, [
      _operator(q3, 'type:chat', key: 'type', value: 'chat'),
    ]);
    expect(withTypeFilters(q3, filters3, {}), '');
  });

  test('a negated type operator survives a chip click', () {
    const q = '-type:chat garden';
    final filters = _filters(q, [
      _operator(q, '-type:chat', key: 'type', value: 'chat', negated: true),
    ]);
    expect(
      withTypeFilters(q, filters, searchTypeChats),
      '-type:chat garden type:chat',
    );
  });

  test('a quoted phrase with type: keeps its words when chips are clicked', () {
    const q = '"say type:chat" type:document';
    final filters = _filters(q, [
      _term(q, '"say type:chat"', value: 'say type:chat'),
      _operator(q, 'type:document', key: 'type', value: 'document'),
    ]);
    expect(
      withTypeFilters(q, filters, searchTypeChats),
      '"say type:chat" type:chat',
    );
    expect(withTypeFilters(q, filters, {}), '"say type:chat"');
  });

  test('a quoted property key keeps its text when chips are clicked', () {
    const q = '"due date":today type:chat';
    final filters = _filters(q, [
      _term(q, '"due date":today', value: 'due date:today'),
      _operator(q, 'type:chat', key: 'type', value: 'chat'),
    ]);
    expect(filters, hasLength(1));
    expect(withTypeFilters(q, filters, {}), '"due date":today');
  });

  test('byte spans cut correctly after multi-byte characters', () {
    const q = 'café type:chat tag:x';
    final filters = _filters(q, [
      _operator(q, 'type:chat', key: 'type', value: 'chat'),
    ]);
    expect(filters.single.start, utf8.encode('café ').length);
    expect(
      withTypeFilters(q, filters, searchTypePages),
      'café tag:x type:document,database',
    );
  });

  test('mismatched token and spec operators read no filters', () {
    // The parser pairs them one to one; a mismatch must not guess.
    const q = 'type:chat';
    final pieces = [_operator(q, 'type:chat', key: 'type', value: 'chat')];
    expect(
      typeFiltersOf(
        q,
        ParsedQuery(
          spec: const SearchSpec(terms: [], operators: [], warnings: []),
          tokens: [for (final p in pieces) p.token],
        ),
      ),
      isEmpty,
    );
  });

  test('highlight words are lower-case, split on spaces, longest first', () {
    expect(highlightTerms(['Garden  Ka', 'ka']), ['garden', 'ka']);
    expect(highlightTerms(['']), isEmpty);
  });
}
