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
    Info(exa_proto::ExascriptClient),
    /// Ask the DB for column metadata; sent after MT_INFO, before MT_META.
    MetaRequest,
    MetaReply(exa_proto::ExascriptMetadata),
    EmitData(exa_proto::ExascriptTableData),
    Next,
    DoneReply,
    CleanupReply,
    FinishedReply,
    CloseError(String),
    PingReply(String),
    /// Return the string result of a single-call function invocation.
    SingleCallReturn(String),
    /// Tell the DB the requested function is not implemented in this container.
    UndefinedCall(String),
}
