//
// Copyright 2025 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

package org.signal.libsignal.net;

import org.signal.libsignal.internal.CompletableFuture;
import org.signal.libsignal.internal.Native;
import org.signal.libsignal.internal.NativeTesting;
import org.signal.libsignal.net.internal.BridgeChatListener;
import org.signal.libsignal.protocol.util.Pair;

/**
 * Represents an unauthenticated (i.e. hopefully anonymous) communication channel with the Chat
 * service.
 *
 * <p>Created by the factory method {@link Network#connectUnauthChat} rather than instantiated
 * directly.
 *
 * <p>Note that a newly-created instance of this class won't be usable for sending messages or
 * receiving events until {@link ChatConnection#start()} is called.
 */
public class UnauthenticatedChatConnection extends ChatConnection {
  private UnauthenticatedChatConnection(
      final TokioAsyncContext tokioAsyncContext,
      long nativeHandle,
      ChatConnectionListener listener,
      Network.Environment ktEnvironment) {
    super(tokioAsyncContext, nativeHandle, listener);
    this.keyTransparencyClient = new KeyTransparencyClient(this, tokioAsyncContext, ktEnvironment);
  }

  private KeyTransparencyClient keyTransparencyClient;

  static CompletableFuture<UnauthenticatedChatConnection> connect(
      final TokioAsyncContext tokioAsyncContext,
      final Network.ConnectionManager connectionManager,
      ChatConnectionListener chatListener) {
    return tokioAsyncContext.guardedMap(
        asyncContextHandle ->
            connectionManager.guardedMap(
                connectionManagerHandle ->
                    Native.UnauthenticatedChatConnection_connect(
                            asyncContextHandle, connectionManagerHandle)
                        .thenApply(
                            nativeHandle ->
                                new UnauthenticatedChatConnection(
                                    tokioAsyncContext,
                                    nativeHandle,
                                    chatListener,
                                    connectionManager.environment()))));
  }

  /**
   * High-level key transparency subsystem client on top using {@code this} to communicate with the
   * chat server.
   *
   * @return an instance of {@link KeyTransparencyClient}
   */
  public KeyTransparencyClient keyTransparencyClient() {
    return this.keyTransparencyClient;
  }

  /**
   * Test-only method to create a {@code UnauthenticatedChatConnection} connected to a fake remote.
   *
   * <p>The returned {@link FakeChatRemote} can be used to send messages to the connection.
   */
  public static Pair<UnauthenticatedChatConnection, FakeChatRemote> fakeConnect(
      final TokioAsyncContext tokioAsyncContext,
      ChatConnectionListener listener,
      Network.Environment ktEnvironment) {

    return tokioAsyncContext.guardedMap(
        asyncContextHandle -> {
          SetChatLaterListenerBridge bridgeListener = new SetChatLaterListenerBridge();
          long fakeChatConnection =
              NativeTesting.TESTING_FakeChatConnection_Create(
                  asyncContextHandle, bridgeListener, "");
          UnauthenticatedChatConnection chat =
              new UnauthenticatedChatConnection(
                  tokioAsyncContext,
                  NativeTesting.TESTING_FakeChatConnection_TakeUnauthenticatedChat(
                      fakeChatConnection),
                  listener,
                  ktEnvironment);
          bridgeListener.setChat(chat);
          FakeChatRemote fakeRemote =
              new FakeChatRemote(
                  tokioAsyncContext,
                  NativeTesting.TESTING_FakeChatConnection_TakeRemote(fakeChatConnection));
          NativeTesting.FakeChatConnection_Destroy(fakeChatConnection);
          return new Pair<>(chat, fakeRemote);
        });
  }

  // Implementing these abstract methods from ChatConnection allows UnauthenticatedChatConnection
  //   to get the implementation of its main functionality (connect, send, etc.)
  //   using the shared implementations of those methods in ChatConnection.
  @Override
  protected CompletableFuture disconnectWrapper(
      long nativeAsyncContextHandle, long nativeChatServiceHandle) {
    return Native.UnauthenticatedChatConnection_disconnect(
        nativeAsyncContextHandle, nativeChatServiceHandle);
  }

  @Override
  protected void startWrapper(long nativeChatConnectionHandle, BridgeChatListener listener) {
    Native.UnauthenticatedChatConnection_init_listener(nativeChatConnectionHandle, listener);
  }

  @Override
  protected CompletableFuture<Object> sendWrapper(
      long nativeAsyncContextHandle,
      long nativeChatConnectionHandle,
      long nativeRequestHandle,
      int timeoutMillis) {
    return Native.UnauthenticatedChatConnection_send(
        nativeAsyncContextHandle, nativeChatConnectionHandle, nativeRequestHandle, timeoutMillis);
  }

  @Override
  protected void release(long nativeChatConnectionHandle) {
    Native.UnauthenticatedChatConnection_Destroy(nativeChatConnectionHandle);
  }
}
