// The board add-on's public API. The app imports only this file.

import '../../registry/addon_registry.dart';
import '../../core/models.dart';
import 'board_view.dart';

export 'board_view.dart' show boardView;

class BoardAddon extends Addon {
  const BoardAddon();
  @override
  String get id => 'board';
  @override
  List<String> get dependsOn => const ['database'];
  @override
  void register(AddonContext ctx) =>
      ctx.views.register(boardView, before: ViewKind.timeline);
}
