import { invoke } from '@tauri-apps/api';
import clsx from 'clsx';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { motion } from 'framer-motion';
import { useNavigate } from 'react-router-dom';
import dayjs from 'dayjs';
import { NymDarkOutlineIcon, NymIcon } from '../../assets';
import { useInAppNotify, useMainDispatch, useMainState } from '../../contexts';
import { useI18nError } from '../../hooks';
import { routes } from '../../router';
import { BackendError, StateDispatch } from '../../types';
import { Button, PageAnim, TextArea } from '../../ui';
import { kvSet } from '../../kvStore';

function AddCredential() {
  const { uiTheme, daemonStatus } = useMainState();
  const [credential, setCredential] = useState('');
  const [error, setError] = useState<string | null>(null);

  const { push } = useInAppNotify();
  const navigate = useNavigate();
  const { t } = useTranslation('addCredential');
  const { tE } = useI18nError();
  const dispatch = useMainDispatch() as StateDispatch;

  const onChange = (credential: string) => {
    setCredential(credential);
    if (credential.length == 0) {
      setError(null);
    }
  };

  const handleClick = () => {
    if (credential.length === 0) {
      return;
    }
    invoke<number | null>('add_credential', { credential: credential.trim() })
      .then((expiry) => {
        if (expiry) {
          const date = dayjs.unix(expiry);
          kvSet<string>('CredentialExpiry', date.toISOString());
          dispatch({ type: 'set-credential-expiry', expiry: date });
        } else {
          console.warn('no expiry date returned from the backend');
        }
        navigate(routes.root);
        push({
          text: t('added-notification'),
          position: 'top',
          closeIcon: true,
        });
      })
      .catch((e: unknown) => {
        const eT = e as BackendError;
        console.log('backend error:', e);
        if (eT.key === 'CredentialExpired' && eT.data?.expiration) {
          // TODO the expiration date format passed from the backend is not ISO_8601
          // So we have to parse it manually
          setError(
            `${tE(eT.key)}: ${dayjs(eT.data.expiration, 'YYYY-MM-DD').format('YYYY-MM-DD')}`,
          );
        } else {
          setError(tE(eT.key));
        }
      });
  };

  return (
    <PageAnim className="h-full flex flex-col justify-end items-center gap-10 select-none cursor-default">
      {uiTheme === 'Dark' ? (
        <NymDarkOutlineIcon className="w-32 h-32" />
      ) : (
        <NymIcon className="w-32 h-32 fill-ghost" />
      )}
      <div className="flex flex-col items-center gap-4 px-4">
        <h1 className="text-2xl dark:text-white">{t('welcome')}</h1>
        <h2 className="text-center dark:text-laughing-jack">
          {t('description1')}
        </h2>
        <p className="text-xs text-center text-dim-gray dark:text-mercury-mist w-5/6">
          {t('description2')}
        </p>
      </div>
      <div className="w-full">
        <TextArea
          value={credential}
          onChange={onChange}
          spellCheck={false}
          resize="none"
          rows={10}
          label={t('input-label')}
          className="sentry-ignore"
        />
        {error ? (
          <motion.div
            initial={{ opacity: 0, x: -10 }}
            animate={{ opacity: 1, x: 0 }}
            transition={{ duration: 0.15, ease: 'easeInOut' }}
            className="text-teaberry h-3"
          >
            {error}
          </motion.div>
        ) : (
          <div className="h-3"></div>
        )}
      </div>
      <Button
        onClick={handleClick}
        disabled={daemonStatus !== 'Ok'}
        className={clsx(
          daemonStatus !== 'Ok' &&
            'opacity-50 disabled:opacity-50 hover:opacity-50',
        )}
      >
        {t('add-button')}
      </Button>
    </PageAnim>
  );
}

export default AddCredential;
