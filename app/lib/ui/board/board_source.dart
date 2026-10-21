import 'package:flutter/widgets.dart';

/// A board's presentation data. IDs belong to the source, not to a database.
class BoardColumn {
  const BoardColumn({
    required this.id,
    required this.title,
    required this.count,
    this.color,
    this.background,
    String? keyLabel,
  }) : keyLabel = keyLabel ?? title;

  final String id, title;
  final int count;
  final Color? color, background;

  /// Presentation key retained by adapters with existing widget contracts.
  final String keyLabel;
}

class BoardCard {
  const BoardCard({
    required this.id,
    required this.title,
    this.peek = const [],
  });
  final String id, title;
  final List<String> peek;
}

/// Supplies a board without requiring any particular storage or note type.
///
/// [cardAt] may start an asynchronous window load and return null meanwhile.
/// [loadedCards] must not fetch more windows: it supplies the current end-drop
/// anchor, including when a source has unloaded gaps. Sources notify listeners
/// when cards, counts, or columns change.
/// The caller owns and disposes the source; the renderer only subscribes.
abstract class BoardSource extends ChangeNotifier {
  List<BoardColumn> get columns;
  int cardCount(String columnId);
  BoardCard? cardAt(String columnId, int index);
  Iterable<BoardCard> loadedCards(String columnId);

  /// Null [afterCardId] places the card first; otherwise it follows that card.
  Future<void> moveCard(
    String cardId,
    String fromColumnId,
    String toColumnId,
    String? afterCardId,
  );
  Future<void> addCard(BuildContext context, String columnId);
  void openCard(BuildContext context, BoardCard card);

  /// Optional editing capabilities. A live query may keep its columns fixed.
  Future<void> Function(String title)? get addColumn => null;
  Future<void> Function(String columnId, String title)? get renameColumn =>
      null;
  Future<void> Function(String columnId, String? afterColumnId)?
  get reorderColumn => null;

  /// Removes a column. Its cards stay as notes and move to the first remaining
  /// column; the source does the relocation.
  /// A caller-supplied columnMenu owns the whole menu, so the kit Delete entry
  /// appears only in the default menu.
  Future<void> Function(String columnId)? get deleteColumn => null;
}

typedef BoardCardBuilder = Widget Function(
  BuildContext context,
  BoardColumn column,
  BoardCard card,
  bool lifted,
);
typedef BoardColumnLabelBuilder = Widget Function(
  BuildContext context,
  BoardColumn column,
);
typedef BoardColumnMenu = void Function(
  BuildContext anchor,
  BoardColumn column,
);
