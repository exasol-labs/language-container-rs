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
}
