//
// Copyright 2014-2016 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

package org.signal.libsignal.protocol.message;

import org.signal.libsignal.internal.NativeHandleGuard;

public interface CiphertextMessage extends NativeHandleGuard.Owner {

  public static final int CURRENT_VERSION = 3;

  public static final int WHISPER_TYPE = 2;
  public static final int PREKEY_TYPE = 3;
  public static final int SENDERKEY_TYPE = 7;
  public static final int PLAINTEXT_CONTENT_TYPE = 8;

  // This should be the worst case (worse than V2).  So not always accurate, but good enough for
  // padding.
  public static final int ENCRYPTED_MESSAGE_OVERHEAD = 53;

  public byte[] serialize();

  public int getType();
}
