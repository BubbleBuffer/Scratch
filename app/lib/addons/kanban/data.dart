// The kanban's stored shape and its pure rules. Storage calls live in
// source.dart; nothing here touches the vault, so the parse rules stay easy
// to read and to test.

import 'dart:convert';
import 'dart:math';

/// The add-on id: the key of this add-on's type-data area on a kanban page.
const kanbanAddonId = 'kanban';

/// The page type of a kanban note.
const kanbanPageType = 'kanban';

/// The columns of a new board, in order.
const kanbanStarterColumns = ['To do', 'Doing', 'Done'];

final _random = Random.secure();
const _idChars = 'abcdefghijklmnopqrstuvwxyz0123456789';

/// A short random id for a column (unique enough within one board).
String newKanbanColumnId() =>
    [for (var i = 0; i < 8; i++) _idChars[_random.nextInt(_idChars.length)]]
        .join();

class KanbanColumn {
  const KanbanColumn({required this.id, required this.title, this.color});
  final String id, title;

  /// A design token name, stored for later use; not drawn yet.
  final String? color;
}

/// The board as stored. Card entries are note ids in column order; a note id
/// may be listed even when its page is missing or in the trash (see the
/// source, which hides such cards without removing them).
class KanbanData {
  const KanbanData({required this.columns, required this.cards});

  final List<KanbanColumn> columns;

  /// Column id to its card page ids. A column without an entry has no cards.
  final Map<String, List<String>> cards;

  static const empty = KanbanData(columns: [], cards: {});

  /// The ids listed in [columnId], or none.
  List<String> cardsOf(String columnId) => cards[columnId] ?? const [];

  /// Parses the JSON of the kanban's type-data area. Anything malformed reads
  /// as missing, so a damaged entry never hides the rest of the board. The
  /// first column that lists a note keeps it; the board's own id is dropped.
  static KanbanData parse(String json, {required String pageId}) {
    final root = _decode(json);
    if (root == null) return empty;

    final columns = <KanbanColumn>[];
    final columnIds = <String>{};
    final rawColumns = root['columns'];
    if (rawColumns is List) {
      for (final item in rawColumns) {
        if (item is! Map) continue;
        final id = item['id'];
        if (id is! String || id.isEmpty || !columnIds.add(id)) continue;
        final title = item['title'];
        final color = item['color'];
        columns.add(
          KanbanColumn(
            id: id,
            title: title is String ? title : '',
            color: color is String && color.isNotEmpty ? color : null,
          ),
        );
      }
    }

    final cards = <String, List<String>>{};
    final seen = <String>{pageId};
    final rawCards = root['cards'];
    if (rawCards is Map) {
      for (final column in columns) {
        final list = rawCards[column.id];
        if (list is! List) continue;
        final ids = <String>[];
        for (final item in list) {
          if (item is String && seen.add(item)) ids.add(item);
        }
        cards[column.id] = ids;
      }
    }
    return KanbanData(columns: columns, cards: cards);
  }

  static Map<String, dynamic>? _decode(String json) {
    try {
      final value = jsonDecode(json);
      return value is Map<String, dynamic> ? value : null;
    } on FormatException {
      return null;
    }
  }
}

/// Where [after] sits in [list] for an insert that follows it: 0 for the
/// front, and the end when [after] is not in the list.
int kanbanPositionAfter(List<String> list, String? after) {
  if (after == null) return 0;
  final index = list.indexOf(after);
  return index < 0 ? list.length : index + 1;
}
