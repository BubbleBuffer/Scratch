// A real NotesShell and Rust vault, driven entirely inside flutter_tester.
@Tags(['drive'])
library;

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/debounce.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/shell/page_stack.dart';

import '../harness.dart';

void main() {
  setUpAll(initCore);
  appTest(
    'headless drive session',
    (tester) async {
      final config = jsonDecode(
        Platform.environment['NOTES_DRIVE_CONFIG'] ?? '{}',
      ) as Map<String, dynamic>;
      final vault = config['vault'] as String? ?? 'fresh';
      final now = config['now'] as String?;
      await pumpApp(
        tester,
        mobile: config['mobile'] == true,
        dark: config['dark'] == true,
        demo: vault == 'demo',
        empty: vault == 'empty',
        fresh: vault == 'fresh',
        copyVault: vault.startsWith('copy:') ? vault.substring(5) : null,
        now: now == null ? null : DateTime.parse(now.replaceFirst(' ', 'T')),
      );
      final semantics = tester.ensureSemantics();
      final driver = _Driver(tester);
      driver.resize(config);
      await settle(tester);
      final initialException = tester.takeException();
      if (initialException != null) {
        throw StateError('app exception: $initialException');
      }
      final commands = StreamController<_Command>();
      final iterator = StreamIterator(commands.stream);
      HttpServer? server;
      final session = File('build/drive/session.json');
      final random = math.Random.secure();
      final token = base64Url.encode(
        List.generate(32, (_) => random.nextInt(256)),
      );
      try {
        await tester.runAsync(() async {
          server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
          server!.listen((request) async {
            request.response.headers.contentType = ContentType.json;
            if (request.method != 'POST' || request.uri.path != '/command') {
              request.response.statusCode = 404;
              request.response.write(
                jsonEncode({'ok': false, 'error': 'POST /command required'}),
              );
              await request.response.close();
              return;
            }
            if (request.headers.value(HttpHeaders.authorizationHeader) !=
                'Bearer $token') {
              request.response.statusCode = 403;
              request.response.write(
                jsonEncode({'ok': false, 'error': 'unauthorized'}),
              );
              await request.response.close();
              return;
            }
            try {
              final body = await utf8.decoder.bind(request).join();
              if (body.length > 1024 * 1024) {
                throw const FormatException('command too large');
              }
              commands.add(
                _Command(request, jsonDecode(body) as Map<String, dynamic>),
              );
            } catch (e) {
              request.response.write(
                jsonEncode({'ok': false, 'error': _oneLine(e)}),
              );
              await request.response.close();
            }
          });
          session.parent.createSync(recursive: true);
          final temporary = File('${session.path}.tmp');
          temporary.writeAsStringSync(
            jsonEncode({'port': server!.port, 'pid': pid, 'token': token}),
          );
          temporary.renameSync(session.path);
        });
        while (true) {
          final available = await tester.runAsync(
            () => iterator.moveNext().timeout(
              const Duration(minutes: 20),
              onTimeout: () => false,
            ),
          );
          if (available != true) break;
          final command = iterator.current;
          Map<String, dynamic> reply;
          try {
            reply = await driver.execute(command.data);
            final exception = tester.takeException();
            if (exception != null) {
              throw StateError('app exception: $exception');
            }
            reply['ok'] = true;
          } catch (e, stack) {
            final exception = tester.takeException();
            reply = {
              'ok': false,
              'error': exception == null
                  ? _oneLine(e)
                  : 'app exception: ${_oneLine(exception)}',
            };
            // Preserve useful diagnostics in the redirected host log.
            debugPrint('$e\n$stack');
          }
          await tester.runAsync(() async {
            command.request.response.write(jsonEncode(reply));
            await command.request.response.close();
          });
          if (command.data['command'] == 'stop') break;
        }
      } finally {
        semantics.dispose();
        await driver.close();
        await tester.runAsync(() async {
          await server?.close(force: true);
          await iterator.cancel();
          await commands.close();
          if (session.existsSync()) {
            final saved = jsonDecode(session.readAsStringSync()) as Map;
            if (saved['token'] == token) session.deleteSync();
          }
        });
      }
    },
    tags: ['drive'],
    timeout: Timeout.none,
  );
}

String _oneLine(Object error) =>
    error.toString().replaceAll(RegExp(r'\s+'), ' ').trim();

class _Command {
  _Command(this.request, this.data);
  final HttpRequest request;
  final Map<String, dynamic> data;
}

class _Entry {
  _Entry(this.id, this.role, this.label, this.rect, this.states, this.value);
  final int id;
  final String role, label;
  final Rect rect;
  final List<String> states;
  final String? value;
  String ref = '';
  Map<String, dynamic> json() => {
    'ref': ref,
    'role': role,
    'label': label,
    'x': rect.left,
    'y': rect.top,
    'width': rect.width,
    'height': rect.height,
    'states': states,
    if (value != null) 'value': value,
  };
}

class _Driver {
  _Driver(this.tester) {
    final shots = Directory('build/drive/shots');
    if (shots.existsSync()) {
      for (final file in shots.listSync()) {
        final number = RegExp(r'^(\d+)-')
            .firstMatch(file.uri.pathSegments.last);
        if (number != null) shot = math.max(shot, int.parse(number[1]!));
      }
    }
  }
  final WidgetTester tester;
  bool mobile = false;
  int shot = 0;
  List<_Entry> lastLook = [];
  TestGesture? mouse;
  Size get size => tester.view.physicalSize / tester.view.devicePixelRatio;

  void resize(Map<String, dynamic> args) {
    if (args['mobile'] != null) mobile = args['mobile'] == true;
    final width =
        (args['width'] as num?)?.toDouble() ?? (mobile ? 390.0 : 1440.0);
    final height =
        (args['height'] as num?)?.toDouble() ?? (mobile ? 844.0 : 900.0);
    if (width <= 0 || height <= 0) throw ArgumentError('size must be positive');
    debugDefaultTargetPlatformOverride = mobile
        ? TargetPlatform.android
        : TargetPlatform.windows;
    tester.view.devicePixelRatio = mobile ? 2 : 1;
    tester.view.physicalSize =
        Size(width, height) * tester.view.devicePixelRatio;
    tester.view.padding = mobile
        ? const FakeViewPadding(top: 48, bottom: 32)
        : FakeViewPadding.zero;
    tester.view.viewPadding = tester.view.padding;
    tester.view.viewInsets = FakeViewPadding.zero;
  }

  List<_Entry> reading() {
    final entries = <_Entry>[];
    final root = tester
        .binding
        .renderViews
        .first
        .owner
        ?.semanticsOwner
        ?.rootSemanticsNode;
    if (root == null) return entries;
    void visit(SemanticsNode node, Matrix4 parentTransform, Rect clip) {
      final transform = parentTransform.clone();
      if (node.transform != null) transform.multiply(node.transform!);
      if (node.parentPaintClipRect != null) {
        clip = clip.intersect(
          // Framework clips are expressed in this node's local coordinates.
          MatrixUtils.transformRect(transform, node.parentPaintClipRect!),
        );
      }
      final data = node.getSemanticsData();
      final flags = data.flagsCollection;
      final rect = MatrixUtils.transformRect(
        transform,
        node.rect,
      ).intersect(clip);
      if (flags.isHidden || rect.isEmpty || !rect.isFinite) return;
      String role = 'text';
      if (flags.isTextField) {
        role = 'textfield';
      } else if (flags.isChecked != ui.CheckedState.none) {
        role = 'checkbox';
      } else if (flags.isToggled != ui.Tristate.none) {
        role = 'toggle';
      } else if (data.role == ui.SemanticsRole.tab) {
        role = 'tab';
      } else if (data.role == ui.SemanticsRole.menuItem ||
          data.role == ui.SemanticsRole.menuItemCheckbox ||
          data.role == ui.SemanticsRole.menuItemRadio) {
        role = 'menuitem';
      } else if (flags.isLink) {
        role = 'link';
      } else if (flags.isButton || data.hasAction(ui.SemanticsAction.tap)) {
        role = 'button';
      } else if (flags.isImage) {
        role = 'image';
      } else if (flags.isHeader) {
        role = 'header';
      }
      final visibleLabel = data.label
          .split('\n')
          .where(
            (line) =>
                !RegExp(r'^row-[0-9a-fA-F-]{32,36}$').hasMatch(line.trim()),
          )
          .join('\n')
          .trim();
      final label = visibleLabel.isNotEmpty ? visibleLabel : data.hint.trim();
      if (label.isNotEmpty || data.value.isNotEmpty || role != 'text') {
        entries.add(
          _Entry(node.id, role, label, rect, [
            if (flags.isChecked == ui.CheckedState.isTrue) 'checked',
            if (flags.isSelected == ui.Tristate.isTrue) 'selected',
            if (flags.isExpanded == ui.Tristate.isTrue) 'expanded',
            if (flags.isFocused == ui.Tristate.isTrue) 'focused',
            if (flags.isEnabled == ui.Tristate.isFalse) 'disabled',
          ], role == 'textfield' ? data.value : null),
        );
      }
      node.visitChildren((child) {
        visit(child, transform, clip);
        return true;
      });
    }

    // The semantics root is in physical pixels; its child includes the DPR
    // transform. Normalize once so readings and target points stay logical.
    final logicalTransform = Matrix4.diagonal3Values(
      1 / tester.view.devicePixelRatio,
      1 / tester.view.devicePixelRatio,
      1,
    );
    visit(root, logicalTransform, Offset.zero & size);
    // Child text already contained verbatim in a button's accessible label is
    // redundant. Keep textfields, whose values are part of their own reading.
    entries.removeWhere(
      (entry) =>
          entry.role == 'text' &&
          entries.any(
            (parent) =>
                parent != entry &&
                parent.role != 'text' &&
                parent.rect.contains(entry.rect.center) &&
                (parent.label == entry.label ||
                    parent.label.split('\n').contains(entry.label)),
          ),
    );
    entries.sort((a, b) {
      final row = a.rect.top.compareTo(b.rect.top);
      return row != 0 ? row : a.rect.left.compareTo(b.rect.left);
    });
    for (var i = 0; i < entries.length; i++) {
      entries[i].ref = 'e${i + 1}';
    }
    return entries;
  }

  Offset target(String value) {
    final coordinates = RegExp(r'^(-?\d+(?:\.\d+)?),\s*(-?\d+(?:\.\d+)?)$')
        .firstMatch(value);
    if (coordinates != null) {
      final point = Offset(
        double.parse(coordinates[1]!),
        double.parse(coordinates[2]!),
      );
      if (!(Offset.zero & size).contains(point)) {
        throw StateError('coordinate outside screen');
      }
      return point;
    }
    final current = reading();
    if (RegExp(r'^e\d+$').hasMatch(value)) {
      final previous = lastLook.where((e) => e.ref == value).toList();
      if (previous.isEmpty) {
        throw StateError('unknown ref $value; use look first');
      }
      final updated = current.where((e) => e.id == previous.single.id).toList();
      if (updated.isEmpty) throw StateError('$value is no longer visible');
      return updated.single.rect.center;
    }
    final matches = current
        .where(
          (e) =>
              e.label == value ||
              e.label.split('\n').contains(value) ||
              e.value == value,
        )
        .toList();
    String candidate(_Entry e) {
      final prior = lastLook.where((p) => p.id == e.id).toList();
      return '[${prior.isEmpty ? e.ref : prior.single.ref}] ${e.role} ${jsonEncode(e.label)}';
    }

    if (matches.length == 1) return matches.single.rect.center;
    if (matches.length > 1) {
      throw StateError(
        'ambiguous ${jsonEncode(value)}: ${matches.map(candidate).join('; ')}',
      );
    }
    current.sort(
      (a, b) => _distance(
        value.toLowerCase(),
        a.label.toLowerCase(),
      ).compareTo(_distance(value.toLowerCase(), b.label.toLowerCase())),
    );
    throw StateError(
      'target ${jsonEncode(value)} not found; closest: ${current.take(5).map(candidate).join('; ')}',
    );
  }

  Future<Map<String, dynamic>> execute(Map<String, dynamic> args) async {
    final command = args['command'] as String;
    final noSettle = args['noSettle'] == true;
    if (command == 'look' && !noSettle) await settle(tester);
    switch (command) {
      case 'ping':
        return {'vaultPath': current.vault.root};
      case 'stop':
        await settle(tester);
        await finishAsync(tester, current.app.pageStore.lastSave);
        await tester.runAsync(current.vault.flush);
      case 'look':
        final label = (args['label'] as String? ?? 'screen').replaceAll(
          RegExp(r'[^a-zA-Z0-9_-]'),
          '-',
        );
        final path = File(
          'build/drive/shots/${(++shot).toString().padLeft(3, '0')}-$label.png',
        ).absolute;
        // Match the golden matcher's ancestor climb from the harness root.
        // Searching descendant boundaries can capture a covered Home route.
        var boundary = tester
            .element(find.byType(Directionality).first)
            .renderObject!;
        while (!boundary.isRepaintBoundary) {
          boundary = boundary.parent!;
        }
        final layer = boundary.debugLayer! as OffsetLayer;
        // The nearest golden boundary can already be in physical pixels.
        // Render exactly the view's physical size without applying DPR twice.
        final rasterRatio =
            tester.view.physicalSize.width / boundary.paintBounds.width;
        await tester.runAsync(() async {
          final image = await layer.toImage(
            boundary.paintBounds,
            pixelRatio: rasterRatio,
          );
          try {
            if (image.width != tester.view.physicalSize.width.round() ||
                image.height != tester.view.physicalSize.height.round()) {
              throw StateError(
                'golden image size ${image.width}x${image.height} '
                'does not match physical view ${tester.view.physicalSize}',
              );
            }
            final bytes = await image.toByteData(
              format: ui.ImageByteFormat.png,
            );
            path.parent.createSync(recursive: true);
            path.writeAsBytesSync(bytes!.buffer.asUint8List());
          } finally {
            image.dispose();
          }
        });
        lastLook = reading();
        final semanticsRoot = tester
            .binding
            .renderViews
            .first
            .owner
            ?.semanticsOwner
            ?.rootSemanticsNode;
        if (semanticsRoot != null) {
          File('${path.path}.semantics.txt')
              .writeAsStringSync(semanticsRoot.toStringDeep());
        }
        return {
          'path': path.path,
          'entries': lastLook.map((e) => e.json()).toList(),
        };
      case 'state':
        final scope = tester.widget<PageStackScope>(
          find.byType(PageStackScope).first,
        );
        final stack = scope.notifier!;
        // Navigator overlay children are laid out in route-stack order, even
        // when covered routes are offstage. Include regular screens as well
        // as page sheets; PageStackNavigator.pages intentionally omits them.
        final routes = <Route<dynamic>>[];
        final accessibleTitles = <Route<dynamic>, String>{};
        for (final element in tester.allElements) {
          final route = _routeOf(element);
          if (route == null ||
              !route.isActive ||
              route.navigator != stack.navigator) {
            continue;
          }
          if (!routes.contains(route)) routes.add(route);
          final widget = element.widget;
          if (widget is Semantics &&
              widget.properties.namesRoute == true &&
              widget.properties.label?.isNotEmpty == true) {
            accessibleTitles[route] = widget.properties.label!;
          }
        }
        if (stack.home != null && !routes.contains(stack.home)) {
          routes.insert(0, stack.home!);
        }
        for (final route in stack.pages) {
          if (!routes.contains(route)) routes.add(route);
        }
        final top = stack.currentRoute;
        if (top != null) {
          routes.remove(top);
          routes.add(top);
        }
        String titleOf(Route<dynamic> route) {
          if (identical(route, stack.home)) return 'Home';
          final id = stack.idOf(route);
          if (id != null) {
            return current.app.pages.meta(id)?.title ??
                stack.titles[id] ??
                'Untitled';
          }
          final accessible = accessibleTitles[route];
          if (accessible != null) return accessible;
          final name = route.settings.name;
          if (name == null || name.isEmpty) return 'Dialog';
          final words = name
              .replaceAll(RegExp(r'[/_-]+'), ' ')
              .trim()
              .split(' ');
          return words
              .where((word) => word.isNotEmpty)
              .map((word) => '${word[0].toUpperCase()}${word.substring(1)}')
              .join(' ');
        }
        final titles = routes.map(titleOf).toList();
        if (titles.isEmpty) titles.add('Home');
        final focused = focusedEditable();
        return {
          'title': titles.last,
          'routes': titles,
          'focused': focused == null
              ? null
              : {
                  'value': focused.controller.text,
                  'label': reading()
                      .where((e) => e.states.contains('focused'))
                      .map((e) => e.label)
                      .join(', '),
                },
          'pendingDebounce': Debounce.pending,
          'pendingBridge': Vault.inFlight.value,
          'vaultPath': current.vault.root,
        };
      case 'resize':
        resize(args);
      case 'theme':
        tester.platformDispatcher.platformBrightnessTestValue =
            args['dark'] == true ? Brightness.dark : Brightness.light;
      case 'wait':
        final ms = (args['ms'] as num).toInt();
        if (ms < 0 || ms > 1200000) {
          throw ArgumentError('wait must be 0..1200000 ms');
        }
        var remaining = ms;
        while (remaining > 0) {
          final step = math.min(remaining, 16);
          await tester.pump(Duration(milliseconds: step));
          await tester.runAsync(() => Future<void>.delayed(Duration.zero));
          remaining -= step;
        }
        return {};
      case 'hover':
        await hover(target(args['target'] as String));
      case 'tap':
      case 'double':
      case 'long':
      case 'right':
        final point = target(args['target'] as String);
        final kind = mobile ? PointerDeviceKind.touch : PointerDeviceKind.mouse;
        if (!mobile) await hover(point);
        if (command == 'long') {
          final gesture = await tester.startGesture(point, kind: kind);
          await tester.pump(const Duration(milliseconds: 650));
          await gesture.up();
        } else {
          await tester.tapAt(
            point,
            kind: kind,
            buttons: command == 'right'
                ? kSecondaryMouseButton
                : kPrimaryButton,
          );
          if (command == 'double') {
            await tester.pump(const Duration(milliseconds: 80));
            await tester.tapAt(point, kind: kind);
          }
        }
      case 'drag':
        await drag(
          target(args['from'] as String),
          target(args['to'] as String),
        );
      case 'scroll':
        final point = target(args['target'] as String);
        final dy = (args['dy'] as num).toDouble();
        if (mobile) {
          // Slow, stationary release avoids ballistic fling.
          final end = Offset(
            point.dx,
            (point.dy - dy).clamp(1.0, size.height - 1),
          );
          await drag(point, end);
        } else {
          await hover(point);
          tester.binding.handlePointerEvent(
            PointerScrollEvent(
              position: point,
              scrollDelta: Offset(0, dy),
              kind: PointerDeviceKind.mouse,
            ),
          );
        }
      case 'type':
        final text = (args['text'] as String).replaceAll(r'\n', '\n');
        if (args['paste'] == true) {
          // All at once: one edit per line.
          final lines = text.split('\n');
          for (var i = 0; i < lines.length; i++) {
            if (i > 0) {
              await key('enter');
              if (!noSettle) await settle(tester);
            }
            insert(lines[i]);
            await tester.pump();
          }
        } else {
          // Like a person: one character per edit, through the same text
          // input path, with a short settle so pickers (@, [[, /) react.
          for (final rune in text.runes) {
            final char = String.fromCharCode(rune);
            if (char == '\n') {
              await key('enter');
            } else {
              insert(char);
            }
            await settleBriefly();
          }
        }
      case 'key':
        await key(args['combo'] as String);
      default:
        throw ArgumentError('unknown command: $command');
    }
    if (!noSettle) await settle(tester);
    return {};
  }

  /// Replaces the focused field's selection with [text], as the platform
  /// text input would.
  void insert(String text) {
    final edit = focusedEditable();
    if (edit == null) throw StateError('no focused text field');
    final value = edit.controller.value;
    final selection = value.selection.isValid
        ? value.selection
        : TextSelection.collapsed(offset: value.text.length);
    tester.testTextInput.updateEditingValue(value.replaced(selection, text));
  }

  /// A typing pause: a few frames plus pending bridge replies, without
  /// waiting for the whole app to go quiet.
  Future<void> settleBriefly() async {
    for (var i = 0; i < 3; i++) {
      await tester.pump(const Duration(milliseconds: 16));
      await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    }
  }

  EditableText? focusedEditable() {
    final editors = tester
        .widgetList<EditableText>(find.byType(EditableText))
        .where((e) => e.focusNode.hasFocus);
    return editors.isEmpty ? null : editors.first;
  }

  Future<void> hover(Offset point) async {
    if (mouse == null) {
      mouse = await tester.createGesture(
        kind: PointerDeviceKind.mouse,
        pointer: 900,
      );
      await mouse!.addPointer(location: point);
    }
    await mouse!.moveTo(point);
    await tester.pump();
  }

  Future<void> drag(Offset from, Offset to) async {
    final gesture = await tester.startGesture(
      from,
      kind: mobile ? PointerDeviceKind.touch : PointerDeviceKind.mouse,
    );
    for (var i = 1; i <= 12; i++) {
      await gesture.moveTo(Offset.lerp(from, to, i / 12)!);
      await tester.pump(const Duration(milliseconds: 25));
    }
    await tester.pump(const Duration(milliseconds: 200));
    await gesture.up();
  }

  Future<void> key(String combo) async {
    final parts = combo.toLowerCase().split('+');
    final modifiers = <LogicalKeyboardKey>[];
    for (final modifier in parts.take(parts.length - 1)) {
      modifiers.add(switch (modifier) {
        'ctrl' || 'control' => LogicalKeyboardKey.controlLeft,
        'shift' => LogicalKeyboardKey.shiftLeft,
        'alt' => LogicalKeyboardKey.altLeft,
        'meta' || 'cmd' || 'win' => LogicalKeyboardKey.metaLeft,
        _ => throw ArgumentError('unknown modifier: $modifier'),
      });
    }
    final named = <String, LogicalKeyboardKey>{
      'enter': LogicalKeyboardKey.enter,
      'escape': LogicalKeyboardKey.escape,
      'esc': LogicalKeyboardKey.escape,
      'tab': LogicalKeyboardKey.tab,
      'backspace': LogicalKeyboardKey.backspace,
      'delete': LogicalKeyboardKey.delete,
      'left': LogicalKeyboardKey.arrowLeft,
      'right': LogicalKeyboardKey.arrowRight,
      'up': LogicalKeyboardKey.arrowUp,
      'down': LogicalKeyboardKey.arrowDown,
      'arrowleft': LogicalKeyboardKey.arrowLeft,
      'arrowright': LogicalKeyboardKey.arrowRight,
      'arrowup': LogicalKeyboardKey.arrowUp,
      'arrowdown': LogicalKeyboardKey.arrowDown,
      'home': LogicalKeyboardKey.home,
      'end': LogicalKeyboardKey.end,
      'pageup': LogicalKeyboardKey.pageUp,
      'pagedown': LogicalKeyboardKey.pageDown,
      'space': LogicalKeyboardKey.space,
      '/': LogicalKeyboardKey.slash,
    };
    final name = parts.last;
    final logical =
        named[name] ??
        (name.length == 1
            ? LogicalKeyboardKey.findKeyByKeyId(name.codeUnitAt(0))
            : null);
    if (logical == null) throw ArgumentError('unknown key: $name');
    try {
      for (final modifier in modifiers) {
        await tester.sendKeyDownEvent(modifier);
      }
      await tester.sendKeyEvent(logical);
    } finally {
      for (final modifier in modifiers.reversed) {
        await tester.sendKeyUpEvent(modifier);
      }
    }
    // Platform text entry is separate from physical key dispatch. Flutter's
    // test engine has no OS IME to turn a printable unmodified key into text.
    if (modifiers.isEmpty && name == '/' && focusedEditable() != null) {
      final value = focusedEditable()!.controller.value;
      tester.testTextInput.updateEditingValue(
        value.replaced(value.selection, '/'),
      );
    }
    await tester.pump();
  }

  Future<void> close() async {
    await mouse?.removePointer();
  }
}

int _distance(String a, String b) {
  var previous = List.generate(b.length + 1, (i) => i);
  for (var i = 1; i <= a.length; i++) {
    final next = <int>[i];
    for (var j = 1; j <= b.length; j++) {
      next.add(
        math.min(
          math.min(next[j - 1] + 1, previous[j] + 1),
          previous[j - 1] + (a[i - 1] == b[j - 1] ? 0 : 1),
        ),
      );
    }
    previous = next;
  }
  return previous.last;
}

/// The route [element] sits in, found without registering a dependency.
///
/// `ModalRoute.of` makes the element depend on the route's scope; called on
/// every element (including the scope's own), that corrupts the dependency
/// set and the next route change fails Flutter's descendant assertion.
ModalRoute<dynamic>? _routeOf(Element element) {
  ModalRoute<dynamic>? found;
  bool check(Element e) {
    if (e.widget.runtimeType.toString() == '_ModalScopeStatus') {
      found = (e.widget as dynamic).route as ModalRoute<dynamic>;
      return false;
    }
    return true;
  }

  if (check(element)) element.visitAncestorElements(check);
  return found;
}
