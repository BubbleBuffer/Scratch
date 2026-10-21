// Page-wide undo and redo. Each entry holds the ops that undo a user action
// and the ops that redo it, plus where the caret was before and after. The
// editor pushes one entry per command (split, move, turn into, marks...)
// and one per run of typing in a block; see PageEditorState.

import 'package:flutter/foundation.dart';

import '../core/models.dart';

/// A block and caret offset to return to.
typedef Caret = (String block, int offset);

class UndoEntry {
  UndoEntry({required this.label, required this.undo, required this.redo, this.before, this.after});

  /// What the action was ("Typing", "Move block", ...).
  final String label;
  final List<BOp> undo;

  /// Growing while the same run of typing continues.
  List<BOp> redo;

  /// Where the caret goes after undo ([before]) or redo ([after]).
  final Caret? before;
  Caret? after;
}

class UndoStack extends ChangeNotifier {
  UndoStack({this.limit = 200});
  final int limit;
  final List<UndoEntry> _undo = [];
  final List<UndoEntry> _redo = [];

  bool get canUndo => _undo.isNotEmpty;
  bool get canRedo => _redo.isNotEmpty;

  /// The entry the next undo would take.
  UndoEntry? get last => _undo.isEmpty ? null : _undo.last;

  void push(UndoEntry e) {
    _undo.add(e);
    if (_undo.length > limit) _undo.removeAt(0);
    _redo.clear();
    notifyListeners();
  }

  /// The entry to undo (its `undo` ops must then be applied), or null.
  UndoEntry? popUndo() {
    if (_undo.isEmpty) return null;
    final e = _undo.removeLast();
    _redo.add(e);
    notifyListeners();
    return e;
  }

  /// The entry to redo (its `redo` ops must then be applied), or null.
  UndoEntry? popRedo() {
    if (_redo.isEmpty) return null;
    final e = _redo.removeLast();
    _undo.add(e);
    notifyListeners();
    return e;
  }

  void clear() {
    _undo.clear();
    _redo.clear();
    notifyListeners();
  }
}
