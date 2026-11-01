// Fabricated search hits for the grouped-results tests and goldens: pages
// from the vault and chats from the agent, with a chat's target JSON.

import 'dart:convert';

import 'package:bb_ui/search.dart' as bb;
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/shell/search_groups.dart' show hitRowId;
import 'package:notes_app/shell/search_widgets.dart';

Hit pageHit(String id, String title, {String snippet = ''}) => Hit(
  id: id,
  title: title,
  snippet: snippet,
  fuzzy: false,
  source: 'vault',
  pageType: 'document',
  target: null,
);

/// A chat hit: [breadcrumb] is the core's display title, [summary] the
/// snippet, and the target carries the session and message range (an empty
/// [firstMessage] and [lastMessage] make a title-only row).
Hit chatHit(
  String id, {
  String breadcrumb = 'BubblePlay cursor › Native drag-and-drop',
  String summary = 'Explorer stalls during the Windows handoff.',
  String level = 'section',
  String sessionId = 'sess-1',
  String firstMessage = 'm-53',
  String lastMessage = 'm-88',
}) => Hit(
  id: id,
  title: breadcrumb,
  snippet: summary,
  fuzzy: false,
  source: 'agent',
  pageType: 'chat',
  target: jsonEncode({
    'sessionId': sessionId,
    'firstMessageId': firstMessage,
    'lastMessageId': lastMessage,
    'level': level,
    'chapterTitle': 'Native drag-and-drop',
    'sectionTitle': 'Chrome live drops pass',
  }),
);

/// The hits the search UI shows right now, in row order: each shared result
/// row mapped back to the [Hit] the overlay body or results list was given.
List<Hit> shownHits(WidgetTester tester) {
  final known = <String, Hit>{
    for (final body in tester.widgetList<SearchPanelBody>(
      find.byType(SearchPanelBody),
    ))
      for (final hit in body.hits ?? const <Hit>[]) hitRowId(hit): hit,
    for (final list in tester.widgetList<SearchResultsList>(
      find.byType(SearchResultsList),
    ))
      for (final section in list.sections)
        for (final hit in section.hits) hitRowId(hit): hit,
  };
  return [
    for (final row in tester.widgetList<bb.SearchResultRow>(
      find.byType(bb.SearchResultRow),
    ))
      known[row.result.id]!,
  ];
}
