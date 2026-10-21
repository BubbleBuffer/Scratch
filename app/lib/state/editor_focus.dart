import 'package:flutter/foundation.dart';
import 'package:flutter/scheduler.dart';

import 'app_state.dart';
import '../src/rust/api/agent.dart' show AgentContextSelection;

/// The visible editor's current selection, shared with the agent composer.
class EditorFocus extends ChangeNotifier {
  static final _instances = Expando<EditorFocus>();
  static EditorFocus forApp(AppState app) => _instances[app] ??= EditorFocus();
  static void disposeFor(AppState app) {
    _instances[app]?.dispose();
    _instances[app] = null;
  }

  String? pageId;
  String pageTitle = '';
  List<String> blockIds = const [];
  String text = '';
  int composerRequest = 0;
  AgentContextSelection? _composerSelection;
  bool _disposed = false;

  void _changed() {
    if (SchedulerBinding.instance.schedulerPhase ==
        SchedulerPhase.persistentCallbacks) {
      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!_disposed) notifyListeners();
      });
    } else if (!_disposed) {
      notifyListeners();
    }
  }

  void update({
    required String pageId,
    required String pageTitle,
    List<String> blockIds = const [],
    String text = '',
  }) {
    if (this.pageId == pageId &&
        this.pageTitle == pageTitle &&
        listEquals(this.blockIds, blockIds) &&
        this.text == text) {
      return;
    }
    this.pageId = pageId;
    this.pageTitle = pageTitle;
    this.blockIds = List.unmodifiable(blockIds);
    this.text = text;
    _changed();
  }

  void clear(String pageId) {
    if (this.pageId != pageId) return;
    this.pageId = null;
    pageTitle = '';
    blockIds = const [];
    text = '';
    _changed();
  }

  void requestComposer() {
    final id = pageId;
    _composerSelection = id == null
        ? null
        : AgentContextSelection(
            pageId: id,
            pageTitle: pageTitle,
            blockIds: List.unmodifiable(blockIds),
            text: text,
          );
    composerRequest++;
    _changed();
  }

  /// Explicit Ask context survives closing its originating editor on mobile.
  AgentContextSelection? takeComposerSelection() {
    final selection = _composerSelection;
    _composerSelection = null;
    return selection;
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
