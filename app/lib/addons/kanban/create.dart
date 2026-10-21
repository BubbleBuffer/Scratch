// Making kanban notes: a new board starts with its three starter columns, and
// each column starts with an empty card list.

import 'dart:convert';

import '../../core/vault.dart';
import 'data.dart';
import 'markdown.dart';

/// Creates a kanban note under [parent] (top level when null) and returns its
/// id. Its columns are the starter columns; it has no cards yet.
Future<String> createKanban(
  Vault vault, {
  String? parent,
  String title = '',
}) async {
  final id = await vault.createPage(
    parent: parent,
    title: title,
    pageType: kanbanPageType,
  );
  final columns = [
    for (final name in kanbanStarterColumns)
      {'id': newKanbanColumnId(), 'title': name},
  ];
  await vault.typeDataSet(id, kanbanAddonId, const [
    'columns',
  ], jsonEncode(columns));
  await vault.typeDataSet(
    id,
    kanbanAddonId,
    const ['cards'],
    jsonEncode({for (final column in columns) column['id']!: const <String>[]}),
  );
  // The readable body follows the board from the start: no cards yet.
  await vault.setTypeMarkdownBody(
    id,
    kanbanReadableBody([
      for (final name in kanbanStarterColumns) KanbanBodyColumn(name, const []),
    ]),
  );
  return id;
}
