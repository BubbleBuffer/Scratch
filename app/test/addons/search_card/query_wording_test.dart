// Unit tests for the per-query wording (chip label and empty line). No bridge:
// the tokens are built by hand in the shape the core's parse_search returns,
// for ASCII queries (token offsets are UTF-8 byte offsets).

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/search_card/query_wording.dart';
import 'package:notes_app/src/rust/api/types.dart';

/// Operator tokens for `key:value` words and `#tag`, text tokens for the rest.
/// A leading `-` marks a negation; its span starts with the minus sign, as
/// the core's spans do.
List<Token> tokensFor(String query) {
  final tokens = <Token>[];
  var offset = 0;
  for (final word in query.split(' ')) {
    final start = offset;
    offset += word.length + 1;
    if (word.isEmpty) continue;
    final negated = word.startsWith('-');
    final body = negated ? word.substring(1) : word;
    if (body.startsWith('#')) {
      tokens.add(_op(start, word.length, 'tag', body.substring(1)));
      continue;
    }
    final colon = body.indexOf(':');
    if (colon > 0 && !body.substring(0, colon).contains('"')) {
      tokens.add(
        _op(start, word.length, body.substring(0, colon), body.substring(colon + 1)),
      );
      continue;
    }
    tokens.add(
      Token(
        start: start,
        end: start + word.length,
        kind: TokenKind(
          kind: SearchTokenKind.text,
          value: word,
          phrase: false,
          key: '',
          message: '',
        ),
      ),
    );
  }
  return tokens;
}

Token _op(int start, int length, String key, String value) => Token(
  start: start,
  end: start + length,
  kind: TokenKind(
    kind: SearchTokenKind.operator_,
    value: value,
    phrase: false,
    key: key,
    message: '',
  ),
);

QueryWording wording(String query, int count) => queryWording(
  query: query,
  tokens: tokensFor(query),
  count: count,
);

void main() {
  group('task lists', () {
    test('a tagged task filter reads as the mockup does, without "open"', () {
      final w = wording('#work type:task sort:updated', 3);
      expect(w.chipLabel, '3 #work tasks');
      expect(w.emptyLine, 'No #work tasks.');
    });

    test('one task is singular', () {
      expect(wording('#work type:task', 1).chipLabel, '1 #work task');
    });

    test('zero tasks reads "no ..." in the chip', () {
      final w = wording('#work type:task', 0);
      expect(w.chipLabel, 'no #work tasks');
      expect(w.emptyLine, 'No #work tasks.');
    });

    test('a done filter that asks for not-done tasks says "open"', () {
      final w = wording('#work done:no', 2);
      expect(w.chipLabel, '2 open #work tasks');
      expect(w.emptyLine, 'No open #work tasks.');
    });

    test('a task filter that excludes done tasks reads "open"', () {
      final w = wording('#work type:task -status:done', 3);
      expect(w.chipLabel, '3 open #work tasks');
      expect(w.emptyLine, 'No open #work tasks.');
      expect(wording('#work type:task -status:done', 1).chipLabel, '1 open #work task');
      expect(wording('#work type:task -status:done', 0).chipLabel, 'no open #work tasks');
    });

    test('a negated status that is not done does not make tasks open', () {
      final w = wording('#work type:task -status:doing', 2);
      expect(w.chipLabel, '2 #work tasks');
    });

    test('a task filter without tags has no tag in the wording', () {
      final w = wording('type:task', 4);
      expect(w.chipLabel, '4 tasks');
      expect(w.emptyLine, 'No tasks.');
    });

    test('a negated task filter does not make the query a task list', () {
      final w = wording('-type:task updated:week', 5);
      expect(w.chipLabel, '5 pages this week');
    });
  });

  group('status columns', () {
    test('a status filter reads "N in <value>"', () {
      final w = wording('in:[[Sprint board]] status:doing', 3);
      expect(w.chipLabel, '3 in Doing');
      expect(w.emptyLine, 'Nothing in Doing.');
    });

    test('one row is still "in Doing", and zero reads "Nothing in Doing."', () {
      expect(wording('status:doing', 1).chipLabel, '1 in Doing');
      expect(wording('status:doing', 0).emptyLine, 'Nothing in Doing.');
    });

    test('the status key ignores case and separators', () {
      final w = wording('STATUS:doing', 2);
      expect(w.chipLabel, '2 in Doing');
    });
  });

  group('pages this week', () {
    test('updated:week with sort reads "N pages this week"', () {
      final w = wording('updated:week -"Week 42" sort:updated', 7);
      expect(w.chipLabel, '7 pages this week');
      expect(w.emptyLine, 'Nothing edited this week.');
    });

    test('one page is singular', () {
      expect(wording('updated:week', 1).chipLabel, '1 page this week');
    });

    test('an extra filter takes the query out of the week wording', () {
      final w = wording('updated:week tag:work', 2);
      expect(w.chipLabel, '2 results · updated:week tag:work');
    });
  });

  group('other queries', () {
    test('a plain query reads "N results · <query>"', () {
      final w = wording('quarterly', 2);
      expect(w.chipLabel, '2 results · quarterly');
      expect(w.emptyLine, 'No results for quarterly.');
    });

    test('one result is singular', () {
      expect(wording('quarterly', 1).chipLabel, '1 result · quarterly');
    });

    test('zero results of a general query use the general empty line', () {
      final w = wording('in:[[Sprint board]]', 0);
      expect(w.chipLabel, '0 results · in:[[Sprint board]]');
      expect(w.emptyLine, 'No results for in:[[Sprint board]].');
    });

    test('the query text is trimmed', () {
      expect(wording('  quarterly  ', 2).emptyLine, 'No results for quarterly.');
    });
  });

  group('normalizeQueryKey', () {
    test('lowercases and drops spaces, underscores and dashes', () {
      expect(normalizeQueryKey('Due Date'), 'duedate');
      expect(normalizeQueryKey('due_date'), 'duedate');
      expect(normalizeQueryKey('Due-Date'), 'duedate');
      expect(normalizeQueryKey('STATUS'), 'status');
    });
  });
}
