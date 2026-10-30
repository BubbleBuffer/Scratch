// The page behind an old targetless inline database block, made on first
// render.

import '../../core/models.dart';
import '../../core/vault.dart';
import '../../ui/database/values.dart';

/// Makes [page] a database: a "Table" view, the
/// title column and a "Tags" multi-select.
Future<void> makeDatabase(Vault vault, String page) async {
  await vault.run((v) => v.makeCollection(page: page), writes: true);
  final schema = await vault.run((v) => v.getSchema(collection: page));
  if (schema.isEmpty) {
    final saved = await vault.run(
        (v) => v.setSchema(
            collection: page,
            schema: const [PropDef(key: '', name: 'Tags', kind: PropKind.multiSelect, options: [], config: '')]),
        writes: true);
    final views = await vault.run((v) => v.listViews(collection: page));
    for (final view in views) {
      if (view.visibleProps.isEmpty) {
        await vault.run((v) => v.updateView(collection: page, view: view.copyWith(visibleProps: [saved.first.key])),
            writes: true);
      }
    }
  }
}

/// Creates the database page of an inline database block in [host] and
/// returns its id.
Future<String> createInlineDatabase(Vault vault, String host) async {
  final id = await vault.run((v) => v.createPage(parent: host, title: ''), writes: true);
  await makeDatabase(vault, id);
  return id;
}
