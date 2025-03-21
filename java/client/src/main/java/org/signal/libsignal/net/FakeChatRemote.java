//
// Copyright 2025 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

package org.signal.libsignal.net;

import org.signal.libsignal.internal.CompletableFuture;
import org.signal.libsignal.internal.NativeHandleGuard;
import org.signal.libsignal.internal.NativeTesting;
import org.signal.libsignal.net.ChatConnection.InternalRequest;
import org.signal.libsignal.protocol.util.Pair;

class FakeChatRemote extends NativeHandleGuard.SimpleOwner {
  private TokioAsyncContext tokioContext;

  FakeChatRemote(TokioAsyncContext tokioContext, long nativeHandle) {
    super(nativeHandle);
    this.tokioContext = tokioContext;
  }

  public CompletableFuture<Pair<InternalRequest, Long>> getNextIncomingRequest() {
    return tokioContext
        .guardedMap(
            asyncContextHandle ->
                this.guardedMap(
                    fakeRemote ->
                        NativeTesting.TESTING_FakeChatRemoteEnd_ReceiveIncomingRequest(
                            asyncContextHandle, fakeRemote)))
        .thenApply(
            sentRequest -> {
              try {
                var httpRequest =
                    new InternalRequest(
                        NativeTesting.TESTING_FakeChatSentRequest_TakeHttpRequest(sentRequest));
                var requestId = NativeTesting.TESTING_FakeChatSentRequest_RequestId(sentRequest);
                return new Pair<>(httpRequest, requestId);
              } finally {
                NativeTesting.FakeChatSentRequest_Destroy(sentRequest);
              }
            });
  }

  @Override
  protected void release(long nativeHandle) {
    NativeTesting.FakeChatRemoteEnd_Destroy(nativeHandle);
  }
}
