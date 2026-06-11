use crate::error::ProtocolError;
use crate::messages::{HostAction, HostEvent};
use crate::meta::UdfMeta;
use exa_proto::{ExascriptRequest, ExascriptResponse, MessageType};

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Handshake,
    Run,
    Cleanup,
    Done,
}

pub struct Protocol {
    connection_id: u64,
    phase: Phase,
    pending_info: Option<exa_proto::ExascriptInfo>,
    single_call_mode: bool,
}

impl Default for Protocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol {
    pub fn new() -> Self {
        Protocol {
            connection_id: 0,
            phase: Phase::Handshake,
            pending_info: None,
            single_call_mode: false,
        }
    }

    /// Feed one response frame; returns the event to handle and (optionally)
    /// an action to send back. Pure: performs no I/O.
    pub fn step(
        &mut self,
        resp: ExascriptResponse,
    ) -> Result<(HostEvent, Option<HostAction>), ProtocolError> {
        let mt = MessageType::try_from(resp.r#type).unwrap_or(MessageType::MtUnknown);
        self.connection_id = resp.connection_id;

        match (mt, self.phase.clone()) {
            (MessageType::MtInfo, Phase::Handshake) => {
                let info = resp
                    .info
                    .ok_or_else(|| ProtocolError::Protocol("MT_INFO missing info field".into()))?;
                self.pending_info = Some(info);
                Ok((HostEvent::Pending, Some(HostAction::MetaRequest)))
            }
            (MessageType::MtMeta, Phase::Handshake) => {
                let meta_pb = resp
                    .meta
                    .ok_or_else(|| ProtocolError::Protocol("MT_META missing meta field".into()))?;
                let info = self
                    .pending_info
                    .take()
                    .ok_or_else(|| ProtocolError::Protocol("MT_META before MT_INFO".into()))?;
                let meta = UdfMeta::from_pb(&meta_pb, &info)?;
                self.single_call_mode = meta.single_call_mode;
                self.phase = Phase::Run;
                Ok((HostEvent::Meta(meta), None))
            }

            (MessageType::MtRun, Phase::Run) => Ok((HostEvent::Run, None)),
            (MessageType::MtEmit, Phase::Run) => Ok((HostEvent::EmitAck, None)),
            (MessageType::MtNext, Phase::Run) => {
                let table = resp
                    .next
                    .map(|n| n.table)
                    .ok_or_else(|| ProtocolError::Protocol("MT_NEXT missing table".into()))?;
                Ok((HostEvent::NextData(table), None))
            }
            // MT_DONE answers MT_NEXT ("input exhausted for this run") and also
            // echoes the client's own MT_DONE. It stays within the run phase:
            // the session only ends when the DB answers a later MT_RUN with
            // MT_CLEANUP.
            (MessageType::MtDone, Phase::Run) => Ok((HostEvent::Done, None)),
            (MessageType::MtReset, Phase::Run) => Ok((HostEvent::Reset, None)),

            // In single-call mode the DB acknowledges the container's MT_RETURN
            // result by echoing MT_RETURN (see the canonical C++ `send_return`).
            // Surface it as `SingleCallAck` so the caller continues the loop with
            // MT_DONE; the session ends only when a later MT_RUN/MT_DONE is
            // answered with MT_CLEANUP.
            (MessageType::MtReturn, Phase::Run) if self.single_call_mode => {
                Ok((HostEvent::SingleCallAck, None))
            }

            // MT_CALL is only valid in single-call mode.
            (MessageType::MtCall, Phase::Run) if self.single_call_mode => {
                let call = resp
                    .call
                    .ok_or_else(|| ProtocolError::Protocol("MT_CALL missing call field".into()))?;
                let fn_id = exa_proto::SingleCallFunctionId::try_from(call.r#fn)
                    .unwrap_or(exa_proto::SingleCallFunctionId::ScFnNil);
                Ok((
                    HostEvent::SingleCall {
                        fn_id,
                        json_arg: call.json_arg,
                        import_spec: call.import_specification,
                        export_spec: call.export_specification,
                    },
                    None,
                ))
            }
            (MessageType::MtCall, _) => Err(ProtocolError::UnexpectedMessage(
                resp.r#type,
                self.phase_name(),
            )),

            // MT_IMPORT carries connection credentials.
            (MessageType::MtImport, _) => {
                let import = resp.import.ok_or_else(|| {
                    ProtocolError::Protocol("MT_IMPORT missing import field".into())
                })?;
                let conn_pb = import.connection_information.ok_or_else(|| {
                    ProtocolError::Protocol(
                        "MT_IMPORT response missing connection_information".into(),
                    )
                })?;
                Ok((
                    HostEvent::ConnInfo(crate::meta::ConnInfo::from_pb(conn_pb)),
                    None,
                ))
            }

            (MessageType::MtCleanup, _) => {
                self.phase = Phase::Cleanup;
                Ok((HostEvent::Cleanup, None))
            }
            (MessageType::MtFinished, _) => {
                self.phase = Phase::Done;
                Ok((HostEvent::Finished, None))
            }
            (MessageType::MtClose, _) => {
                let msg = resp.close.and_then(|c| c.exception_message);
                self.phase = Phase::Done;
                Ok((HostEvent::Close(msg), None))
            }
            (MessageType::MtPingPong, _) => {
                let meta_info = resp.ping.map(|p| p.meta_info).unwrap_or_default();
                Ok((
                    HostEvent::Ping(meta_info.clone()),
                    Some(HostAction::PingReply(meta_info)),
                ))
            }
            (MessageType::MtTryAgain, _) => Ok((HostEvent::TryAgain, None)),

            _ => Err(ProtocolError::UnexpectedMessage(
                resp.r#type,
                self.phase_name(),
            )),
        }
    }

    pub fn connection_id(&self) -> u64 {
        self.connection_id
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Handshake => "Handshake",
            Phase::Run => "Run",
            Phase::Cleanup => "Cleanup",
            Phase::Done => "Done",
        }
    }

    fn base_request(&self, mt: MessageType) -> ExascriptRequest {
        ExascriptRequest {
            r#type: mt as i32,
            connection_id: self.connection_id,
            ..Default::default()
        }
    }

    /// Build the MT_CLIENT request (sent at connection start).
    pub fn client_request(&self, client_name: &str) -> ExascriptRequest {
        ExascriptRequest {
            client: Some(exa_proto::ExascriptClient {
                client_name: client_name.to_string(),
                meta_info: None,
            }),
            ..self.base_request(MessageType::MtClient)
        }
    }

    /// Build the MT_META request (sent after MT_INFO to ask for column
    /// metadata).
    ///
    /// The request envelope carries no metadata payload — it is a bare MT_META
    /// carrying the connection id, and the DB replies with the column
    /// definitions in its MT_META response.
    pub fn meta_request(&self) -> ExascriptRequest {
        self.base_request(MessageType::MtMeta)
    }

    /// Build an MT_RUN request (open the run phase).
    pub fn run_request(&self) -> ExascriptRequest {
        self.base_request(MessageType::MtRun)
    }

    /// Build an MT_NEXT request (ask for more input data).
    pub fn next_request(&self) -> ExascriptRequest {
        self.base_request(MessageType::MtNext)
    }

    /// Build an MT_EMIT request carrying output data.
    pub fn emit_request(&self, table: exa_proto::ExascriptTableData) -> ExascriptRequest {
        ExascriptRequest {
            emit: Some(exa_proto::ExascriptEmitDataReq { table }),
            ..self.base_request(MessageType::MtEmit)
        }
    }

    /// Build an MT_DONE request (no more data to emit).
    pub fn done_request(&self) -> ExascriptRequest {
        self.base_request(MessageType::MtDone)
    }

    /// Build an MT_CLEANUP reply.
    pub fn cleanup_reply(&self) -> ExascriptRequest {
        self.base_request(MessageType::MtCleanup)
    }

    /// Build an MT_FINISHED reply.
    pub fn finished_reply(&self) -> ExascriptRequest {
        self.base_request(MessageType::MtFinished)
    }

    /// Build an MT_CLOSE request, optionally carrying an exception message.
    pub fn close_request(&self, exception_msg: Option<String>) -> ExascriptRequest {
        ExascriptRequest {
            close: Some(exa_proto::ExascriptClose {
                exception_message: exception_msg,
            }),
            ..self.base_request(MessageType::MtClose)
        }
    }

    /// Build an MT_PING_PONG reply echoing the received meta_info.
    pub fn ping_reply(&self, meta_info: &str) -> ExascriptRequest {
        ExascriptRequest {
            ping: Some(exa_proto::ExascriptPing {
                meta_info: meta_info.to_string(),
            }),
            ..self.base_request(MessageType::MtPingPong)
        }
    }

    /// Build an MT_CLOSE carrying an F-UDF-CL-RUST-#### prefixed error message.
    pub fn error_close_request(&self, code: u32, message: &str) -> ExascriptRequest {
        let full_msg = format!("F-UDF-CL-RUST-{:04}: {}", code, message);
        self.close_request(Some(full_msg))
    }

    /// Build an MT_RETURN request carrying the result of a single-call invocation.
    pub fn return_request(&self, result: String) -> ExascriptRequest {
        ExascriptRequest {
            call_result: Some(exa_proto::ExascriptReturnReq { result }),
            ..self.base_request(MessageType::MtReturn)
        }
    }

    /// Build an MT_UNDEFINED_CALL request telling the DB the function is not available.
    pub fn undefined_call_request(&self, remote_fn: &str) -> ExascriptRequest {
        ExascriptRequest {
            undefined_call: Some(exa_proto::ExascriptUndefinedCallReq {
                remote_fn: remote_fn.to_string(),
            }),
            ..self.base_request(MessageType::MtUndefinedCall)
        }
    }

    /// Build an MT_IMPORT request asking for connection credentials.
    ///
    /// The DB replies with an MT_IMPORT response carrying
    /// `connection_information`. Pass an empty `script_name` when requesting
    /// the default connect-back credentials (the DB ignores the field for
    /// `PB_IMPORT_CONNECTION_INFORMATION` requests).
    pub fn import_connection_request(&self, script_name: &str) -> ExascriptRequest {
        ExascriptRequest {
            import: Some(exa_proto::ExascriptImportReq {
                script_name: script_name.to_string(),
                kind: Some(exa_proto::ImportType::PbImportConnectionInformation as i32),
            }),
            ..self.base_request(MessageType::MtImport)
        }
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
