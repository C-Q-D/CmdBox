//! stdout/stderr 快速 Drain、增量 UTF-8 解码和有界输出 Batch。
//!
//! 两个 Reader 线程只负责尽快读取 Pipe，并以非阻塞方式把字节交给协调器；队列满时丢弃
//! 当前实时片段并累计字节数，绝不让外部进程因消费者缓慢而阻塞。协调器在观察入口分配
//! 全局 sequence，只承诺重建自身观察顺序，不声称还原两个独立 OS Pipe 的真实写入时序。

use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::process::windows::managed_process::CapturedOutput;

/// Reader 到协调器的最大待处理片段数。
const INGRESS_CAPACITY: usize = 128;
/// 调用方实时输出队列的最大 Batch 数。
const DELIVERY_CAPACITY: usize = 32;
/// 单个 Reader 每次从 Pipe 读取的字节数。
const READ_CHUNK_BYTES: usize = 8 * 1024;
/// 达到该字节数时立即 flush 当前 Batch。
const BATCH_BYTES: usize = 32 * 1024;
/// 低频输出最多等待该时间即 flush。
const BATCH_INTERVAL: Duration = Duration::from_millis(33);

/// 输出所属的原始进程流。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    /// 标准输出。
    Stdout,
    /// 标准错误。
    Stderr,
}

/// 协调器观察到的一个连续同流文本片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputFragment {
    /// 协调器入口分配的全局单调 sequence。
    pub sequence: u64,
    /// 当前文本来自 stdout 或 stderr。
    pub stream: OutputStream,
    /// 已经增量解码的纯文本。
    pub text: String,
}

/// 一次有界实时投递的有序输出集合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputBatch {
    /// 保持协调器观察顺序的片段；只合并相邻同流文本。
    pub fragments: Vec<OutputFragment>,
    /// 本 Batch 之前因内部或交付队列已满而丢弃的实时字节数。
    pub dropped_bytes_before: u64,
}

/// 当前进程输出的接收端与 Reader/协调器后台任务所有权。
pub struct OutputCapture {
    /// 调用方接收有界实时 Batch 的通道。
    receiver: Receiver<OutputBatch>,
    /// 保持后台任务句柄，Drop 时允许线程自行在 Pipe EOF 后退出。
    _workers: Vec<thread::JoinHandle<()>>,
    /// 尚未成功随 Batch 投递的累计实时丢弃字节。
    dropped_bytes: Arc<AtomicU64>,
}

impl OutputCapture {
    /// 立即启动两个 Reader 和一个协调器，并返回已经绑定的有界接收端。
    pub fn start(output: CapturedOutput) -> Self {
        let (ingress_sender, ingress_receiver) = mpsc::sync_channel(INGRESS_CAPACITY);
        let (delivery_sender, receiver) = mpsc::sync_channel(DELIVERY_CAPACITY);
        let dropped_bytes = Arc::new(AtomicU64::new(0));
        let (stdout, stderr) = output.into_readers();
        let stdout_worker = spawn_reader(
            stdout,
            OutputStream::Stdout,
            ingress_sender.clone(),
            Arc::clone(&dropped_bytes),
        );
        let stderr_worker = spawn_reader(
            stderr,
            OutputStream::Stderr,
            ingress_sender,
            Arc::clone(&dropped_bytes),
        );
        let coordinator_dropped_bytes = Arc::clone(&dropped_bytes);
        let coordinator_worker = thread::spawn(move || {
            coordinate_output(ingress_receiver, delivery_sender, coordinator_dropped_bytes)
        });
        Self {
            receiver,
            _workers: vec![stdout_worker, stderr_worker, coordinator_worker],
            dropped_bytes,
        }
    }

    /// 返回接收端引用，让调用方按自己的线程模型消费 Batch。
    pub fn receiver(&self) -> &Receiver<OutputBatch> {
        &self.receiver
    }

    /// 返回当前尚未成功随 Batch 告知调用方的丢弃实时字节数。
    pub fn pending_dropped_bytes(&self) -> u64 {
        self.dropped_bytes.load(Ordering::Relaxed)
    }
}

/// Reader 传给协调器的内部消息。
enum IngressMessage {
    /// Reader 已完成增量解码的文本块。
    Text {
        /// 文本所属进程流。
        stream: OutputStream,
        /// 不含跨 Chunk 半个字符的完整纯文本。
        text: String,
        /// 该文本对应的原始字节数，用于丢弃统计。
        original_bytes: usize,
    },
    /// 一个 Reader 已经 Drain 到 EOF。
    Finished {
        /// 已经结束的进程流。
        stream: OutputStream,
    },
}

/// 启动一个只负责快速 Drain 的 Reader 线程。
fn spawn_reader<R>(
    mut reader: R,
    stream: OutputStream,
    sender: SyncSender<IngressMessage>,
    dropped_bytes: Arc<AtomicU64>,
) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = vec![0_u8; READ_CHUNK_BYTES];
        let mut decoder = Utf8StreamDecoder::default();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(length) => {
                    let (text, consumed_bytes) = decoder.push(&buffer[..length], false);
                    if !text.is_empty() {
                        let message = IngressMessage::Text {
                            stream,
                            text,
                            original_bytes: consumed_bytes,
                        };
                        match sender.try_send(message) {
                            Ok(()) => {}
                            Err(TrySendError::Full(IngressMessage::Text {
                                original_bytes,
                                ..
                            })) => {
                                dropped_bytes.fetch_add(original_bytes as u64, Ordering::Relaxed);
                            }
                            Err(TrySendError::Disconnected(_)) => {
                                // 消费链断开后仍继续读取 Pipe，只丢弃实时输出直到 EOF。
                            }
                            Err(TrySendError::Full(IngressMessage::Finished { .. })) => {
                                unreachable!()
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
        let (tail, consumed_bytes) = decoder.push(&[], true);
        if !tail.is_empty() {
            let _ = sender.send(IngressMessage::Text {
                stream,
                text: tail,
                original_bytes: consumed_bytes,
            });
        }
        // EOF 必须可靠送达协调器，否则它无法结束；协调器本身不等待慢消费者，因此此处的
        // 短暂阻塞只用于排空内部有界队列，不会形成 UI → Reader 的长期背压。
        let _ = sender.send(IngressMessage::Finished { stream });
    })
}

/// 汇总两个 Reader，在入口分配 sequence 并按容量或时间形成有界 Batch。
fn coordinate_output(
    receiver: Receiver<IngressMessage>,
    delivery: SyncSender<OutputBatch>,
    dropped_bytes: Arc<AtomicU64>,
) {
    let mut fragments = Vec::new();
    let mut batch_bytes = 0_usize;
    let mut next_sequence = 0_u64;
    let mut finished_streams = 0_u8;
    let mut deadline = Instant::now() + BATCH_INTERVAL;
    while finished_streams < 2 {
        let timeout = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(timeout) {
            Ok(IngressMessage::Text {
                stream,
                text,
                original_bytes,
            }) => {
                push_fragment(&mut fragments, &mut next_sequence, stream, text);
                batch_bytes += original_bytes;
            }
            Ok(IngressMessage::Finished { stream: _stream }) => {
                finished_streams += 1;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if batch_bytes >= BATCH_BYTES || Instant::now() >= deadline || finished_streams == 2 {
            flush_batch(&delivery, &mut fragments, &dropped_bytes);
            batch_bytes = 0;
            deadline = Instant::now() + BATCH_INTERVAL;
        }
    }
}

/// 加入一个协调器观察片段，仅把相邻且同流的文本合并到同一 sequence。
fn push_fragment(
    fragments: &mut Vec<OutputFragment>,
    next_sequence: &mut u64,
    stream: OutputStream,
    text: String,
) {
    if let Some(last) = fragments.last_mut() {
        if last.stream == stream {
            last.text.push_str(&text);
            return;
        }
    }
    fragments.push(OutputFragment {
        sequence: *next_sequence,
        stream,
        text,
    });
    *next_sequence += 1;
}

/// 非阻塞投递当前 Batch；队列已满时记录丢弃字节，不反向阻塞协调器或 Reader。
fn flush_batch(
    delivery: &SyncSender<OutputBatch>,
    fragments: &mut Vec<OutputFragment>,
    dropped_bytes: &AtomicU64,
) {
    if fragments.is_empty() {
        return;
    }
    let batch = OutputBatch {
        fragments: std::mem::take(fragments),
        dropped_bytes_before: dropped_bytes.swap(0, Ordering::Relaxed),
    };
    if let Err(TrySendError::Full(batch)) = delivery.try_send(batch) {
        let bytes = batch
            .fragments
            .iter()
            .map(|fragment| fragment.text.len() as u64)
            .sum::<u64>()
            + batch.dropped_bytes_before;
        dropped_bytes.fetch_add(bytes, Ordering::Relaxed);
    }
}

/// 保留跨读取块的不完整 UTF-8 尾部，并对确定无效的字节输出替换字符。
#[derive(Default)]
struct Utf8StreamDecoder {
    /// 尚未形成完整 UTF-8 字符的尾部字节。
    pending: Vec<u8>,
}

impl Utf8StreamDecoder {
    /// 增量解码新字节；EOF 时把剩余无效尾部转换为替换字符。
    fn push(&mut self, bytes: &[u8], eof: bool) -> (String, usize) {
        let available_bytes = self.pending.len() + bytes.len();
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    output.push_str(text);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    if valid > 0 {
                        output.push_str(unsafe {
                            // SAFETY: `valid_up_to` 保证该前缀是合法 UTF-8。
                            std::str::from_utf8_unchecked(&self.pending[..valid])
                        });
                        self.pending.drain(..valid);
                    }
                    if let Some(error_length) = error.error_len() {
                        output.push('\u{FFFD}');
                        self.pending.drain(..error_length);
                    } else if eof {
                        output.push('\u{FFFD}');
                        self.pending.clear();
                        break;
                    } else {
                        break;
                    }
                }
            }
        }
        let consumed_bytes = available_bytes - self.pending.len();
        (output, consumed_bytes)
    }
}

#[cfg(test)]
mod tests {
    //! 输出协调器的跨流顺序与增量解码测试。

    use std::sync::atomic::AtomicU64;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::{
        coordinate_output, IngressMessage, OutputCapture, OutputStream, Utf8StreamDecoder,
        DELIVERY_CAPACITY,
    };
    use crate::execution::artifact::{MaterializedScript, RenderedScript};
    use crate::process::windows::managed_process::ManagedProcess;
    use crate::process::windows::runner::WindowsPowerShellRunner;

    /// 验证中文字符跨读取块时不会被逐块 lossily 解码损坏。
    #[test]
    fn decodes_utf8_character_split_across_chunks() {
        let bytes = "中".as_bytes();
        let mut decoder = Utf8StreamDecoder::default();
        assert_eq!(decoder.push(&bytes[..2], false), (String::new(), 0));
        assert_eq!(decoder.push(&bytes[2..], false), ("中".to_owned(), 3));

        let mut invalid_tail_decoder = Utf8StreamDecoder::default();
        assert_eq!(
            invalid_tail_decoder.push(&[0xE4], false),
            (String::new(), 0)
        );
        assert_eq!(
            invalid_tail_decoder.push(&[], true),
            ("\u{FFFD}".to_owned(), 1)
        );
    }

    /// 验证协调器只合并相邻同流片段，不会把 stdout 跨过 stderr 重排。
    #[test]
    fn preserves_coordinator_observation_order_across_streams() {
        let (ingress_sender, ingress_receiver) = mpsc::sync_channel(8);
        let (delivery_sender, delivery_receiver) = mpsc::sync_channel(DELIVERY_CAPACITY);
        for (stream, bytes) in [
            (OutputStream::Stdout, b"A".as_slice()),
            (OutputStream::Stderr, b"B".as_slice()),
            (OutputStream::Stdout, b"C".as_slice()),
        ] {
            ingress_sender
                .send(IngressMessage::Text {
                    stream,
                    text: String::from_utf8(bytes.to_vec()).expect("测试文本应为 UTF-8"),
                    original_bytes: bytes.len(),
                })
                .expect("应发送测试片段");
        }
        for stream in [OutputStream::Stdout, OutputStream::Stderr] {
            ingress_sender
                .send(IngressMessage::Finished { stream })
                .expect("应结束测试流");
        }
        coordinate_output(
            ingress_receiver,
            delivery_sender,
            Arc::new(AtomicU64::new(0)),
        );
        let batch = delivery_receiver.recv().expect("应收到最终 Batch");
        let observed = batch
            .fragments
            .iter()
            .map(|fragment| (fragment.sequence, fragment.stream, fragment.text.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            observed,
            vec![
                (0, OutputStream::Stdout, "A"),
                (1, OutputStream::Stderr, "B"),
                (2, OutputStream::Stdout, "C"),
            ]
        );
    }

    /// 验证真实 Windows PowerShell 的 stdout/stderr 中文可以被两个 Reader Drain 并解码。
    #[test]
    fn captures_real_powershell_stdout_and_stderr_as_text() {
        let script = "$utf8 = New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding = $utf8; $stdin = [Console]::In.ReadToEnd(); [Console]::Out.WriteLine('标准输出中文'); [Console]::Out.WriteLine(\"stdin=$($stdin.Length)\"); [Console]::Error.WriteLine('标准错误中文')";
        let runner = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");
        let rendered = RenderedScript::windows_powershell(script);
        let artifact = MaterializedScript::create(rendered).expect("应创建测试脚本");
        let launch = runner.process_launch(artifact, &std::env::temp_dir());
        let mut prepared = ManagedProcess::prepare(launch).expect("应准备受管 PowerShell");
        let capture = OutputCapture::start(prepared.take_output().expect("应取得输出 Pipe"));
        let process = prepared.resume().expect("应恢复受管 PowerShell");

        assert_eq!(process.wait().expect("PowerShell 应自然退出"), 0);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stdout = String::new();
        let mut stderr = String::new();
        while Instant::now() < deadline {
            match capture.receiver().recv_timeout(Duration::from_millis(100)) {
                Ok(batch) => {
                    for fragment in batch.fragments {
                        match fragment.stream {
                            OutputStream::Stdout => stdout.push_str(&fragment.text),
                            OutputStream::Stderr => stderr.push_str(&fragment.text),
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        assert!(stdout.contains("标准输出中文"), "实际 stdout：{stdout:?}");
        assert!(
            stdout.contains("stdin=0"),
            "stdin 应立即得到 EOF：{stdout:?}"
        );
        assert!(stderr.contains("标准错误中文"), "实际 stderr：{stderr:?}");
    }

    /// 验证完全不消费实时队列时，高频输出仍会结束且以 dropped 计数保持内存有界。
    #[test]
    fn slow_consumer_does_not_block_high_volume_process_output() {
        let script = "$utf8 = New-Object System.Text.UTF8Encoding($false); [Console]::OutputEncoding = $utf8; 1..200000 | ForEach-Object { [Console]::Out.WriteLine('0123456789abcdef') }";
        let runner = WindowsPowerShellRunner::resolve().expect("系统应提供 Windows PowerShell");
        let rendered = RenderedScript::windows_powershell(script);
        let artifact = MaterializedScript::create(rendered).expect("应创建测试脚本");
        let launch = runner.process_launch(artifact, &std::env::temp_dir());
        let mut prepared = ManagedProcess::prepare(launch).expect("应准备受管 PowerShell");
        let capture = OutputCapture::start(prepared.take_output().expect("应取得输出 Pipe"));
        let process = prepared.resume().expect("应恢复受管 PowerShell");
        let started = Instant::now();

        assert_eq!(process.wait().expect("高频输出进程不应被实时队列阻塞"), 0);
        assert!(started.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(100));
        let batches = capture.receiver().try_iter().collect::<Vec<_>>();
        assert!(batches.len() <= DELIVERY_CAPACITY);
        let reported_dropped = batches
            .iter()
            .map(|batch| batch.dropped_bytes_before)
            .sum::<u64>()
            + capture.pending_dropped_bytes();
        assert!(reported_dropped > 0, "实时队列满时应记录被丢弃字节");
    }
}
