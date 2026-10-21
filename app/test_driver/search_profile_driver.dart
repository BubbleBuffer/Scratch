// Driver for integration_test/search_profile_test.dart (flutter drive --profile).
// Saves the responseData as build/search_perf/search_perf.json, also when the
// test fails, so partial samples and the failing state are kept. Prints the
// stage summary per route from the same data.

// This is a CLI driver; we print results as part of the interface.
// ignore_for_file: avoid_print

import 'package:integration_test/integration_test_driver.dart';

Future<void> main() => integrationDriver(
  responseDataCallback: _save,
  writeResponseOnFailure: true,
);

Future<void> _save(Map<String, dynamic>? data) async {
  if (data == null) {
    print('search perf: the app returned no response data');
    return;
  }
  await writeResponseData(
    data,
    testOutputFilename: 'search_perf',
    destinationDirectory: 'build/search_perf',
  );
  print('search perf: wrote build/search_perf/search_perf.json');
  print(
    'search perf: status=${data['status']}, run=${data['run']}, '
    'platform=${data['platform']}, flutter=${_sdk(data, 'flutter')}, '
    'dart=${_sdk(data, 'dart')}, refreshRateHz=${data['refreshRateHz']}',
  );
  print('search perf: bridge=${data['bridge']}');
  final routes = data['routes'];
  if (routes is! Map) return;
  for (final entry in routes.entries) {
    final route = entry.value as Map<String, dynamic>;
    print(
      'search perf ${entry.key}: n=${route['n']}, '
      'p50=${_ms(route['p50Ms'])} ms, p95=${_ms(route['p95Ms'])} ms, '
      'Flutter ${_sdk(data, 'flutter')}',
    );
    final load = route['loadAverage'];
    if (load is Map) {
      print('  load average (1/5/15m): ${load['before']} -> ${load['after']}');
    }
    final stages = route['stages'];
    if (stages is Map) {
      for (final stage in stages.entries) {
        final summary = stage.value as Map<String, dynamic>;
        print(
          '  ${(stage.key as String).padRight(16)} '
          'n=${summary['n']}, p50=${_ms(summary['p50Ms'])} ms, '
          'p95=${_ms(summary['p95Ms'])} ms',
        );
      }
    }
    final calls = route['calls'];
    if (calls is Map) {
      for (final kind in calls.entries) {
        final counts = (kind.value as List).cast<int>();
        final low = counts.reduce((a, b) => a < b ? a : b);
        final high = counts.reduce((a, b) => a > b ? a : b);
        print(
          '  calls ${(kind.key as String).padRight(16)} per sample: '
          'min=$low, max=$high, n=${counts.length}',
        );
      }
    }
    final frames = route['frames'];
    if (frames is Map) {
      print(
        '  frames: matched ${frames['matched']}/${frames['samples']}, '
        'max per sample ${frames['maxPerSample']}',
      );
    }
  }
  final semantics = data['semantics'];
  if (semantics is List) {
    print('search perf stage meanings:');
    for (final line in semantics) {
      print('  $line');
    }
  }
}

String? _sdk(Map<String, dynamic> data, String key) {
  final sdk = data['sdk'];
  return sdk is Map ? '${sdk[key]}' : null;
}

String _ms(Object? value) => value is num ? value.toStringAsFixed(2) : '-';
