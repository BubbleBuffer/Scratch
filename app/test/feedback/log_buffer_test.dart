import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/feedback/log_buffer.dart';

void main() {
  late void Function(FlutterErrorDetails)? savedFlutter;
  late bool Function(Object, StackTrace)? savedPlatform;

  setUp(() {
    savedFlutter = FlutterError.onError;
    savedPlatform = PlatformDispatcher.instance.onError;
  });

  tearDown(() {
    FlutterError.onError = savedFlutter;
    PlatformDispatcher.instance.onError = savedPlatform;
  });

  group('ring buffer', () {
    test('keeps the last 200 lines, oldest first', () {
      final buffer = LogBuffer();
      for (var i = 1; i <= 201; i++) {
        buffer.addLine('line $i');
      }
      expect(buffer.lines, hasLength(200));
      expect(buffer.lines.first.text, 'line 2');
      expect(buffer.lines.last.text, 'line 201');
    });

    test('lines are timestamped', () {
      final buffer = LogBuffer();
      final before = DateTime.now();
      buffer.addLine('hello');
      final after = DateTime.now();
      expect(buffer.lines.single.time.isBefore(before), isFalse);
      expect(buffer.lines.single.time.isAfter(after), isFalse);
    });

    test('keeps the last 20 errors, oldest first', () {
      final buffer = LogBuffer();
      for (var i = 1; i <= 21; i++) {
        buffer.addError(StateError('error $i'), null, source: 'flutter');
      }
      expect(buffer.errors, hasLength(20));
      expect(buffer.errors.first.error, 'Bad state: error 2');
      expect(buffer.errors.last.error, 'Bad state: error 21');
    });

    test('a null stack trace is stored as an empty string', () {
      final buffer = LogBuffer();
      buffer.addError(StateError('no stack'), null, source: 'platform');
      expect(buffer.errors.single.stack, '');
      expect(buffer.errors.single.source, 'platform');
    });

    test('lists are read-only', () {
      final buffer = LogBuffer();
      buffer.addLine('one');
      expect(
        () => buffer.lines.add(LogLine(DateTime(2026), 'x')),
        throwsUnsupportedError,
      );
    });
  });

  group('FlutterError hook', () {
    test('records the error with its stack and calls the previous handler', () {
      final buffer = LogBuffer();
      final seen = <FlutterErrorDetails>[];
      FlutterError.onError = seen.add;

      installErrorHooks(buffer: buffer);
      final details = FlutterErrorDetails(
        exception: StateError('boom'),
        stack: StackTrace.fromString('#0 fake (package:app/fake.dart:1:1)'),
      );
      FlutterError.onError!(details);

      final recorded = buffer.errors.single;
      expect(recorded.source, 'flutter');
      expect(recorded.error, 'Bad state: boom');
      expect(recorded.stack, contains('package:app/fake.dart:1:1'));
      expect(seen, [details]);
    });
  });

  group('PlatformDispatcher hook', () {
    test('records the error and returns the previous handler result', () {
      final buffer = LogBuffer();
      final calls = <Object>[];
      PlatformDispatcher.instance.onError = (error, stack) {
        calls.add(error);
        return true;
      };

      installErrorHooks(buffer: buffer);
      final handled = PlatformDispatcher.instance.onError!(
        StateError('native'),
        StackTrace.fromString('#0 native (native.dart:2:2)'),
      );

      expect(handled, isTrue);
      expect(calls.single, isA<StateError>());
      expect(buffer.errors.single.source, 'platform');
      expect(buffer.errors.single.error, 'Bad state: native');
      expect(buffer.errors.single.stack, contains('native.dart:2:2'));
    });

    test('returns false when there was no previous handler', () {
      final buffer = LogBuffer();
      PlatformDispatcher.instance.onError = null;

      installErrorHooks(buffer: buffer);
      final handled = PlatformDispatcher.instance.onError!(
        StateError('unhandled'),
        StackTrace.empty,
      );

      expect(handled, isFalse);
      expect(buffer.errors.single.error, 'Bad state: unhandled');
    });
  });

  test('installing twice records each error once', () {
    final buffer = LogBuffer();
    final flutterSeen = <FlutterErrorDetails>[];
    final platformSeen = <Object>[];
    FlutterError.onError = flutterSeen.add;
    PlatformDispatcher.instance.onError = (error, stack) {
      platformSeen.add(error);
      return false;
    };

    installErrorHooks(buffer: buffer);
    installErrorHooks(buffer: buffer);

    FlutterError.onError!(FlutterErrorDetails(exception: StateError('f')));
    PlatformDispatcher.instance.onError!(StateError('p'), StackTrace.empty);

    expect(buffer.errors, hasLength(2));
    expect(buffer.errors.map((e) => e.source), ['flutter', 'platform']);
    expect(flutterSeen, hasLength(1));
    expect(platformSeen, hasLength(1));
  });
}
