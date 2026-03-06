# Module 9 Tasks: Observability & Telemetry

- [x] Integrate `tracing` and `tracing-subscriber`.
- [x] Define `TelemetryProvider`.
- [x] Implement distributed span passing across `mpsc` thread boundaries.
- [x] Export structure logic matching OTLP guidelines.
- [x] Ensure non-blocking file writes for telemetry traces.

## Notes/Issues
- Integrated `tracing` ecosystem with `tracing-subscriber` and `tracing-appender` for high-performance logging.
- Defined `TelemetryProvider` and implemented `OtlpTelemetryProvider` to mimic structured OTLP logging mapping to JSON files.
- Ensured non-blocking, asynchronous file I/O writes avoiding overhead during distributed tracing using `tracing_appender::non_blocking()`.
- Implemented `wrap_with_span` function manually passing `Span` variables through mpsc channels and using `Instrument` trait.
