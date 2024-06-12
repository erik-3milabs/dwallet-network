// Copyright (c) dWallet Labs, Ltd.
// SPDX-License-Identifier: BSD-3-Clause-Clear

use sui_types::messages_signature_mpc::SignatureMPCSessionID;
use std::collections::{HashMap, HashSet};
use rand::rngs::OsRng;
use sui_types::base_types::{EpochId, ObjectRef};
use signature_mpc::twopc_mpc_protocols::{AdditivelyHomomorphicDecryptionKeyShare, GroupElement, PartyID, Result, DecryptionPublicParameters, DKGDecentralizedPartyOutput, DecentralizedPartyPresign, initiate_decentralized_party_sign, SecretKeyShareSizedNumber, message_digest, PublicNonceEncryptedPartialSignatureAndProof, DecryptionKeyShare, AdjustedLagrangeCoefficientSizedNumber, decrypt_signature_decentralized_party_sign, PaillierModulusSizedNumber, ProtocolContext, Commitment, SignatureThresholdDecryptionParty, SignaturePartialDecryptionProofVerificationParty, Value, Hash};
use std::convert::TryInto;
use std::mem;

#[derive(Default)]
pub(crate) enum SignRound {
    FirstRound {
        signature_threshold_decryption_round_parties: Vec<SignatureThresholdDecryptionParty>,
        signature_partial_decryption_proof_verification_round_parties: Vec<SignaturePartialDecryptionProofVerificationParty>
    },
    #[default]
    None,
}

impl SignRound {
    pub(crate) fn new(
        tiresias_public_parameters: DecryptionPublicParameters,
        tiresias_key_share_decryption_key_share: SecretKeyShareSizedNumber,
        epoch: EpochId,
        party_id: PartyID,
        parties: HashSet<PartyID>,
        session_id: SignatureMPCSessionID,
        messages: Vec<Vec<u8>>,
        dkg_output: DKGDecentralizedPartyOutput,
        public_nonce_encrypted_partial_signature_and_proofs: Vec<PublicNonceEncryptedPartialSignatureAndProof<ProtocolContext>>,
        presigns: Vec<DecentralizedPartyPresign>,
        hash: Hash,
    ) -> Result<(Self, (
            Vec<(PaillierModulusSizedNumber, PaillierModulusSizedNumber)>, 
            Vec<AdditivelyHomomorphicDecryptionKeyShare::PartialDecryptionProof>
        ))> {
        let sign_mpc_parties_per_message = initiate_decentralized_party_sign(
            tiresias_key_share_decryption_key_share,
            tiresias_public_parameters.clone(),
            //epoch,
            party_id,
            parties.clone(),
            //session_id,
            dkg_output,
            presigns.clone(), 
            public_nonce_encrypted_partial_signature_and_proofs.clone(),
        )?;

        let (
                (decryption_shares, signature_threshold_decryption_round_parties), 
                (decryption_shares_proofs, signature_partial_decryption_proof_verification_round_parties)
            ): ((Vec<_>, Vec<_>), (Vec<_>, Vec<_>)) = 
            messages.iter()
                .zip(sign_mpc_party_per_message.iter())
                .zip(public_nonce_encrypted_partial_signature_and_proofs.clone().into_iter())
                .map(|((m, (decrypt_party, proof_party)), public_nonce_encrypted_partial_signature_and_proof)| {
            let m = message_digest(m, &hash);
            (
                decrypt_party
                    .partially_decrypt_encrypted_signature_parts_prehash(
                        m,
                        public_nonce_encrypted_partial_signature_and_proof,
                        &mut OsRng,
                    ),
                proof_party
                    .prove_correct_signature_partial_decryption(&mut OsRng)
            )
        }).collect::<Result<Vec<(
            ((PaillierModulusSizedNumber, PaillierModulusSizedNumber), SignatureThresholdDecryptionParty),
            (AdditivelyHomomorphicDecryptionKeyShare::PartialDecryptionProof, SignaturePartialDecryptionProofVerificationParty)
        )>>>()?.into_iter().unzip();

        Ok((
            SignRound::FirstRound {
                signature_threshold_decryption_round_parties,
                signature_partial_decryption_proof_verification_round_parties
            },
            (
                decryption_shares,
                decryption_shares_proofs,
            )
        ))
    }

    pub(crate) fn complete_round(
        &mut self,
        state: SignState
    ) -> Result<SignRoundCompletion> {
        let round = mem::take(self);
        match round {
            SignRound::FirstRound { 
                signature_threshold_decryption_round_parties, 
                signature_partial_decryption_proof_verification_round_parties
            } => {
                let signatures_s = decrypt_signature_decentralized_party_sign(
                    state.messages.unwrap(), 
                    state.tiresias_public_parameters.clone(), 
                    state.decryption_shares.clone(),
                    state.decryption_shares_proofs.clone(),
                    state.public_nonce_encrypted_partial_signature_and_proofs.clone().unwrap(), 
                    signature_threshold_decryption_round_parties,
                    signature_partial_decryption_proof_verification_round_parties
                )?;

                Ok(SignRoundCompletion::Output(signatures_s))            }
            _ => Ok(SignRoundCompletion::None)
        }


    }
}


pub(crate) enum SignRoundCompletion {
    Output(Vec<Vec<u8>>),
    None,
}

#[derive(Clone)]
pub(crate) struct SignState {
    epoch: EpochId,
    party_id: PartyID,
    parties: HashSet<PartyID>,
    aggregator_party_id: PartyID,
    tiresias_public_parameters: DecryptionPublicParameters,

    messages: Option<Vec<Vec<u8>>>,
    public_nonce_encrypted_partial_signature_and_proofs: Option<Vec<PublicNonceEncryptedPartialSignatureAndProof<ProtocolContext>>>,

    decryption_shares: HashMap<PartyID, Vec<(PaillierModulusSizedNumber, PaillierModulusSizedNumber)>>,
    decryption_shares_proofs: HashMap<PartyID, Vec<DecryptionKeyShare::PartialDecryptionProof>>
}

impl SignState {
    pub(crate) fn new(
        tiresias_public_parameters: DecryptionPublicParameters,
        epoch: EpochId,
        party_id: PartyID,
        parties: HashSet<PartyID>,
        session_id: SignatureMPCSessionID,
    ) -> Self {
        let aggregator_party_id = ((u64::from_be_bytes((&session_id.0[0..8]).try_into().unwrap()) % parties.len() as u64) + 1) as PartyID;

        Self {
            epoch,
            party_id,
            parties,
            aggregator_party_id,
            tiresias_public_parameters,
            messages: None,
            public_nonce_encrypted_partial_signature_and_proofs: None,
            decryption_shares: HashMap::new(),
            decryption_shares_proofs: HashMap::new()
        }
    }

    pub(crate) fn set(
        &mut self,
        messages: Vec<Vec<u8>>,
        public_nonce_encrypted_partial_signature_and_proofs: Vec<PublicNonceEncryptedPartialSignatureAndProof<ProtocolContext>>,
    ) {
        self.messages = Some(messages);
        self.public_nonce_encrypted_partial_signature_and_proofs = Some(public_nonce_encrypted_partial_signature_and_proofs);
    }

    pub(crate) fn insert_first_round(
        &mut self,
        party_id: PartyID,
        message: (Vec<(PaillierModulusSizedNumber, PaillierModulusSizedNumber)>, Vec<DecryptionKeyShare::PartialDecryptionProof>),
    ) -> Result<()> {
        let (shares, proofs) = message;
        let _ = self
            .decryption_shares
            .insert(party_id, shares);
        let _ = self
            .decryption_shares_proofs
            .insert(party_id, proofs);
        Ok(())
    }

    pub(crate) fn ready_for_complete_first_round(&self, round: &SignRound) -> bool {
        match round {
            SignRound::FirstRound { .. } if self.decryption_shares.len() == self.parties.len() && self.decryption_shares_proofs.len() == self.parties.len() && self.party_id == self.aggregator_party_id => true,
            _ => false
        }
    }
}
