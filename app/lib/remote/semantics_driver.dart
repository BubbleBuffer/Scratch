// Reads and drives the running UI through the semantics tree: the `tree`,
// `tap` and `type` commands of the app remote control.
//
// Ids are `Semantics.identifier` when set, else `n<semantics node id>`.
// Positions are logical pixels of the window, the same space as the
// screenshot at pixel ratio 1.

import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/semantics.dart';

import 'remote_handler.dart';

/// One listed node.
class UiNode {
  UiNode({
    required this.id,
    required this.nodeId,
    required this.role,
    required this.label,
    required this.value,
    required this.hint,
    required this.actions,
    required this.enabled,
    required this.focused,
    required this.rect,
  });

  final String id;
  final int nodeId;
  final String role;
  final String label;
  final String value;
  final String hint;
  final List<String> actions;
  final bool enabled;
  final bool focused;
  final Rect rect;

  Map<String, Object?> toJson() => {
    'id': id,
    'role': role,
    'label': label,
    'value': value,
    'hint': hint,
    'actions': actions,
    'enabled': enabled,
    'focused': focused,
    'rect': {
      'x': _round(rect.left),
      'y': _round(rect.top),
      'w': _round(rect.width),
      'h': _round(rect.height),
    },
  };

  static double _round(double v) => (v * 10).roundToDouble() / 10;
}

const _listedActions = <String, ui.SemanticsAction>{
  'tap': ui.SemanticsAction.tap,
  'longPress': ui.SemanticsAction.longPress,
  'increase': ui.SemanticsAction.increase,
  'decrease': ui.SemanticsAction.decrease,
  'setText': ui.SemanticsAction.setText,
  'focus': ui.SemanticsAction.focus,
  'dismiss': ui.SemanticsAction.dismiss,
};

class SemanticsDriver {
  /// [owner] and [view] default to the app's first window. [nextFrame]
  /// completes once a new frame has built; tests pump instead.
  SemanticsDriver({
    SemanticsOwner? Function()? owner,
    ui.FlutterView? Function()? view,
    Future<void> Function()? nextFrame,
  }) : _owner = owner ?? _defaultOwner,
       _view = view ?? _defaultView,
       _nextFrame = nextFrame ?? _defaultNextFrame;

  final SemanticsOwner? Function() _owner;
  final ui.FlutterView? Function() _view;
  final Future<void> Function() _nextFrame;
  SemanticsHandle? _handle;

  static SemanticsOwner? _defaultOwner() {
    final views = RendererBinding.instance.renderViews;
    if (views.isEmpty) return null;
    return views.first.owner?.semanticsOwner ??
        RendererBinding.instance.rootPipelineOwner.semanticsOwner;
  }

  static ui.FlutterView? _defaultView() {
    final views = RendererBinding.instance.renderViews;
    return views.isEmpty ? null : views.first.flutterView;
  }

  static Future<void> _defaultNextFrame() {
    SchedulerBinding.instance.scheduleFrame();
    return SchedulerBinding.instance.endOfFrame;
  }

  /// Keeps the semantics tree built; idempotent.
  void enable() => _handle ??= SemanticsBinding.instance.ensureSemantics();

  void disable() {
    _handle?.dispose();
    _handle = null;
  }

  /// Visible nodes with a label, value, hint or action, in reading order.
  List<UiNode> snapshot() {
    final root = _owner()?.rootSemanticsNode;
    final view = _view();
    if (root == null || view == null) return const [];
    final dpr = view.devicePixelRatio;
    final window = Offset.zero & (view.physicalSize / dpr);
    final entries = <UiNode>[];
    final used = <String>{};

    void visit(SemanticsNode node, Matrix4 parent, Rect clip) {
      final transform = parent.clone();
      if (node.transform != null) transform.multiply(node.transform!);
      var childClip = clip;
      if (node.parentPaintClipRect != null) {
        childClip = clip.intersect(
          MatrixUtils.transformRect(transform, node.parentPaintClipRect!),
        );
      }
      final data = node.getSemanticsData();
      final flags = data.flagsCollection;
      final rect = MatrixUtils.transformRect(
        transform,
        node.rect,
      ).intersect(childClip);
      if (flags.isHidden || rect.isEmpty || !rect.isFinite) return;
      final actions = [
        for (final e in _listedActions.entries)
          if (data.hasAction(e.value)) e.key,
      ];
      final label = data.label.trim();
      final hint = data.hint.trim();
      final role = _role(data, flags, actions);
      if (label.isNotEmpty ||
          data.value.isNotEmpty ||
          hint.isNotEmpty ||
          actions.isNotEmpty) {
        var id = data.identifier.isNotEmpty ? data.identifier : 'n${node.id}';
        if (!used.add(id)) id = 'n${node.id}';
        used.add(id);
        entries.add(
          UiNode(
            id: id,
            nodeId: node.id,
            role: role,
            label: label,
            value: data.value,
            hint: hint,
            actions: actions,
            enabled: flags.isEnabled != ui.Tristate.isFalse,
            focused: flags.isFocused == ui.Tristate.isTrue,
            rect: rect,
          ),
        );
      }
      node.visitChildren((child) {
        visit(child, transform, childClip);
        return true;
      });
    }

    // The semantics root is in physical pixels; its child carries the pixel
    // ratio, so scale once to get logical pixels.
    visit(root, Matrix4.diagonal3Values(1 / dpr, 1 / dpr, 1), window);

    // A text child whose words are already in an enclosing control's label
    // adds nothing.
    entries.removeWhere(
      (e) =>
          e.role == 'text' &&
          entries.any(
            (p) =>
                !identical(p, e) &&
                p.role != 'text' &&
                p.rect.contains(e.rect.center) &&
                (p.label == e.label || p.label.split('\n').contains(e.label)),
          ),
    );
    entries.sort((a, b) {
      final row = a.rect.top.compareTo(b.rect.top);
      return row != 0 ? row : a.rect.left.compareTo(b.rect.left);
    });
    return entries;
  }

  static String _role(
    SemanticsData data,
    ui.SemanticsFlags flags,
    List<String> actions,
  ) {
    if (flags.isTextField) return 'textfield';
    if (flags.isChecked != ui.CheckedState.none) return 'checkbox';
    if (flags.isToggled != ui.Tristate.none) return 'switch';
    if (flags.isSlider ||
        actions.contains('increase') ||
        actions.contains('decrease')) {
      return 'slider';
    }
    if (flags.isLink) return 'link';
    if (flags.isButton || actions.contains('tap')) return 'button';
    if (flags.isImage) return 'image';
    if (flags.isHeader) return 'header';
    if (data.label.isNotEmpty || data.value.isNotEmpty) return 'text';
    return 'other';
  }

  UiNode _find(String id) {
    for (final n in snapshot()) {
      if (n.id == id) return n;
    }
    throw RemoteError(404, 'no element with id "$id" in the current tree');
  }

  SemanticsOwner _requireOwner() {
    final owner = _owner();
    if (owner == null) throw const RemoteError(500, 'no semantics tree yet');
    return owner;
  }

  void tap(String id) {
    final node = _find(id);
    if (!node.actions.contains('tap')) {
      throw RemoteError(400, 'element "$id" has no tap action');
    }
    _requireOwner().performAction(node.nodeId, ui.SemanticsAction.tap);
  }

  /// Focuses the field, waits a frame (a field offers setText only while
  /// focused) and sets its text. With [replace] false the text is appended.
  Future<void> type(String id, String text, {required bool replace}) async {
    var node = _find(id);
    if (node.role != 'textfield') {
      throw RemoteError(400, 'element "$id" is not a text field');
    }
    final owner = _requireOwner();
    if (!node.focused) {
      if (node.actions.contains('focus')) {
        owner.performAction(node.nodeId, ui.SemanticsAction.focus);
      } else if (node.actions.contains('tap')) {
        owner.performAction(node.nodeId, ui.SemanticsAction.tap);
      }
      await _nextFrame();
      node = _find(id);
    }
    if (!node.actions.contains('setText')) {
      throw RemoteError(400, 'field "$id" does not accept text (read-only?)');
    }
    final next = replace ? text : '${node.value}$text';
    owner.performAction(node.nodeId, ui.SemanticsAction.setText, next);
  }
}
