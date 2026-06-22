use crate::meta::UdfMeta;

/// High-level event delivered to the runtime after protocol decoding.
#[derive(Debug)]
pub enum HostEvent {
    /// A response was consumed but no actionable event is surfaced yet
    /// (e.g. MT_INFO is buffered until the following MT_META arrives).
    Pending,
    Meta(UdfMeta),
    Run,
    /// The DB acknowledged an `MT_EMIT` by echoing it back.
    EmitAck,
    NextData(exa_proto::ExascriptTableData),
    Done,
    Cleanup,
    /// The DB acknowledged the container's single-call MT_RETURN result by
    /// echoing MT_RETURN. The container continues the single-call loop (closing
    /// the run with MT_DONE); the session ends only on a later MT_CLEANUP.
    SingleCallAck,
    Finished,
    Close(Option<String>),
    Ping(String),
    TryAgain,
    Reset,
    /// The DB requests a synchronous single-call function invocation.
    SingleCall {
        fn_id: exa_proto::SingleCallFunctionId,
        json_arg: Option<String>,
        import_spec: Option<exa_proto::ImportSpecificationRep>,
        export_spec: Option<exa_proto::ExportSpecificationRep>,
    },
    /// The DB returned connection credentials via MT_IMPORT.
    ConnInfo(crate::meta::ConnInfo),
}

/// Action the runtime wants to take, encoded back to a protobuf request.
#[derive(Debug)]
pub enum HostAction {
    /// Ask the DB for column metadata; sent after MT_INFO, before MT_META.
    MetaRequest,
    PingReply(String),
}
