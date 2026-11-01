// Compatibility renderer for stored inline database blocks. The `/` menu no
// longer offers them (View lists a page's child pages instead). Old
// targetless database blocks retain their original lazy creation path.

import 'package:flutter/widgets.dart';

import '../create.dart';
import '../database_view.dart';
import '../../../registry/block_registry.dart';
import '../../../state/app_state.dart';
import '../../../ui/icon_x.dart';
import '../../../ui/ui.dart';

/// The core's kind name of an inline database block.
const databaseKind = 'database';

/// Blocks whose database page is being created (a rebuild must not create a
/// second one).
final _creating = <String>{};

Widget _body(BlockContext b) {
  final n = b.node;
  final target = n.target;
  if (target == null || target.isEmpty) {
    if (!_creating.contains(n.id)) {
      final host = b.host;
      _creating.add(n.id);
      final vault = b.context.app.vault;
      createInlineDatabase(vault, host.pageId).then((id) {
        host.setTarget(n, id);
      }).whenComplete(() => _creating.remove(n.id));
    }
    return SizedBox(
      height: 40,
      child: Align(
        alignment: Alignment.centerLeft,
        child: Text('Creating…', style: ts(b.type.body, b.colors.textTertiary)),
      ),
    );
  }
  return LayoutBuilder(
    builder: (context, box) => DatabaseView(
      key: ValueKey('inline-db-$target'),
      collection: target,
      inline: true,
      width: box.maxWidth,
    ),
  );
}

double _handleTop(BlockContext b) => 6;

const databaseBlock = BlockType(
  kind: databaseKind,
  label: 'Inline view',
  hint: 'Pages shown inline.',
  iconX: Ix.table,
  aliases: 'database table inline db collection',
  editable: false,
  acceptsChildren: false,
  body: _body,
  handleTop: _handleTop,
  inMenu: false,
);
