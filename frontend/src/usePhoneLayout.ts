import { useEffect, useState } from 'react';
import { PHONE_QUERY, isPhoneLayout } from './layout';

/** Whether the phone layout (below 768 px) is on, following the window as it resizes or rotates. */
export function usePhoneLayout(): boolean {
  const [phone, setPhone] = useState(isPhoneLayout);
  useEffect(() => {
    const mq = typeof window !== 'undefined' ? window.matchMedia?.(PHONE_QUERY) : undefined;
    if (!mq) return;
    const on = () => setPhone(mq.matches);
    on();
    mq.addEventListener?.('change', on);
    return () => mq.removeEventListener?.('change', on);
  }, []);
  return phone;
}
