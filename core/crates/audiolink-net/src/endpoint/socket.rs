//! 把实际 UDP 套接字的释放通知交给 shutdown；不在音频收发路径加锁或分配。
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, UdpPoller};
use tokio::sync::oneshot;

pub(super) fn track(
    inner: Arc<dyn AsyncUdpSocket>,
) -> (Arc<dyn AsyncUdpSocket>, oneshot::Receiver<()>) {
    let (sender, receiver) = oneshot::channel();
    let socket = Arc::new(TrackedSocket {
        inner,
        _released: Released(Some(sender)),
    });
    (socket, receiver)
}

#[derive(Debug)]
struct Released(Option<oneshot::Sender<()>>);

impl Drop for Released {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[derive(Debug)]
struct TrackedSocket {
    // 字段按声明顺序析构：先释放真实套接字，再发送通知。
    inner: Arc<dyn AsyncUdpSocket>,
    _released: Released,
}

#[derive(Debug)]
struct TrackedPoller {
    // poller 也可能持有真实 socket；先销毁它，再释放跟踪句柄，避免提前报“已释放”。
    inner: Pin<Box<dyn UdpPoller>>,
    _socket: Arc<TrackedSocket>,
}

impl UdpPoller for TrackedPoller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.get_mut().inner.as_mut().poll_writable(cx)
    }
}

impl AsyncUdpSocket for TrackedSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(TrackedPoller {
            inner: self.inner.clone().create_io_poller(),
            _socket: self,
        })
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        self.inner.try_send(transmit)
    }

    fn poll_recv(
        &self,
        cx: &mut Context<'_>,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        self.inner.poll_recv(cx, bufs, meta)
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn max_transmit_segments(&self) -> usize {
        self.inner.max_transmit_segments()
    }

    fn max_receive_segments(&self) -> usize {
        self.inner.max_receive_segments()
    }

    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[tokio::test]
    async fn release_notification_includes_io_pollers_and_the_native_socket() {
        let native = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = native.local_addr().unwrap();
        let native = quinn::default_runtime()
            .unwrap()
            .wrap_udp_socket(native)
            .unwrap();
        let (socket, mut released) = track(native);
        let poller = socket.clone().create_io_poller();
        drop(socket);
        assert!(matches!(
            released.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert!(std::net::UdpSocket::bind(addr).is_err());
        drop(poller);
        released.await.unwrap();
        std::net::UdpSocket::bind(addr).expect("释放通知之后必须可立即绑定");
    }
}
