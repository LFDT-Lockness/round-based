//! # DKLs23 Distributed Key Generation
//!
//! An implementation of the n-of-n Distributed Key Generation (DKG) protocol
//! from the DKLs23 paper (Doerner, Kondi, Lee, Shelat — "Threshold ECDSA from
//! ECDSA Assumptions: The Multiparty Case", IEEE S&P 2023,
//! <https://eprint.iacr.org/2023/765>).
//!
//! Each of `n` parties jointly generates a shared ECDSA key pair such that:
//! - No single party learns the full secret key `x`
//! - Each party holds an additive share `x_i` where `x = Σ x_i`
//! - All parties learn the combined public key `X = x · G`
//!
//! ## Protocol Overview
//!
//! The DKG runs in three rounds:
//!
//! 1. **Commit**: each party samples a secret share `x_i`, computes public share
//!    `X_i = x_i · G`, prepares a Schnorr commitment `A_i`, and broadcasts
//!    `H(sid ‖ i ‖ rid_i ‖ X_i ‖ A_i ‖ u_i)`.
//!
//! 2. **Decommit**: each party opens the commitment by broadcasting
//!    `(rid_i, X_i, A_i, u_i)`. All parties verify every commitment.
//!
//! 3. **Prove**: each party computes a Schnorr proof-of-knowledge of `x_i`
//!    under the joint random challenge `rid = ⊕ rid_j`, and broadcasts the proof.
//!    All parties verify all proofs.
//!
//! The output for party `i` is a [`KeyShare`] containing the secret share `x_i`,
//! the combined public key `X`, and all parties' public shares `{X_j}`.
//!
//! ## Example
//!
//! ```rust
//! # use generic_ec::curves::Secp256k1;
//! # use dkls23_keygen::*;
//! # fn main() {
//! let n: u16 = 3;
//! let mut rng = rand_dev::DevRng::new();
//!
//! let key_shares: Vec<KeyShare<Secp256k1>> = round_based::sim::run_with_setup(
//!     core::iter::repeat_with(|| rng.fork()).take(n.into()),
//!     |i, party, rng| keygen::<Secp256k1, sha2::Sha256, _, _>(party, i, n, rng),
//! )
//! .unwrap()
//! .expect_ok()
//! .into_iter()
//! .collect();
//!
//! // All parties agree on the same public key
//! let pk = key_shares[0].public_key;
//! for share in &key_shares {
//!     assert_eq!(share.public_key, pk);
//! }
//!
//! // Secret shares reconstruct the full secret key
//! use generic_ec::{Point, Scalar};
//! let x: Scalar<Secp256k1> = key_shares.iter().map(|s| s.secret_share).sum();
//! assert_eq!(Point::generator() * x, pk);
//! # }
//! ```
//!
//! ## References
//!
//! - DKLs23 paper: <https://eprint.iacr.org/2023/765>, Section 5 (Key Generation)
//! - Based on the Pedersen DKG pattern with Schnorr proof of knowledge

#![no_std]
#![forbid(unused_crate_dependencies, missing_docs)]

#[cfg(test)]
extern crate std;

extern crate alloc;

mod _unused_deps {
    use digest as _;
    use generic_array as _;
    #[cfg(test)]
    use hex as _;
    #[cfg(test)]
    use rand as _;
}

use alloc::vec::Vec;

use digest::Digest;
use generic_ec::{Curve, NonZero, Point, Scalar, SecretScalar};
use generic_ec_zkp::schnorr_pok;
use rand_core::{CryptoRng, RngCore};
use round_based::{
    MsgId,
    mpc::{Mpc, MpcExecution},
};
use serde::{Deserialize, Serialize};

/// Domain separation tags for structured hashing
macro_rules! tag {
    ($name:tt) => {
        concat!("dkls23.keygen.", $name)
    };
}

// ---------------------------------------------------------------------------
// Protocol messages
// ---------------------------------------------------------------------------

/// Top-level protocol message
#[derive(round_based::ProtocolMsg, Clone, Serialize, Deserialize)]
#[serde(bound = "")]
pub enum Msg<E: Curve, D: Digest> {
    /// Round 1: hash commitment
    Round1(MsgRound1<D>),
    /// Round 2: decommitment (public data)
    Round2(MsgRound2<E>),
    /// Round 3: Schnorr proof of knowledge
    Round3(MsgRound3<E>),
}

/// Round 1 message — commitment `V_i = H(sid ‖ i ‖ rid_i ‖ X_i ‖ A_i ‖ u_i)`
#[derive(Clone, Serialize, Deserialize, udigest::Digestable)]
#[serde(bound = "")]
#[udigest(bound = "")]
#[udigest(tag = tag!("round1"))]
pub struct MsgRound1<D: Digest> {
    /// Hash commitment
    #[udigest(as_bytes)]
    pub commitment: digest::Output<D>,
}

/// Round 2 message — decommitment revealing public data
#[derive(Clone, Serialize, Deserialize, udigest::Digestable)]
#[serde(bound = "")]
#[udigest(bound = "")]
#[udigest(tag = tag!("round2"))]
pub struct MsgRound2<E: Curve> {
    /// Random identifier contribution `rid_i`
    #[serde(with = "hex_or_bytes")]
    #[udigest(as_bytes)]
    pub rid: [u8; 32],
    /// Public key share `X_i = x_i · G`
    pub public_share: NonZero<Point<E>>,
    /// Schnorr commitment `A_i = α_i · G`
    pub schnorr_commit: schnorr_pok::Commit<E>,
    /// Decommitment nonce `u_i`
    #[serde(with = "hex_or_bytes")]
    #[udigest(as_bytes)]
    pub decommit_nonce: [u8; 32],
}

/// Round 3 message — Schnorr proof of knowledge of `x_i`
#[derive(Clone, Serialize, Deserialize)]
#[serde(bound = "")]
pub struct MsgRound3<E: Curve> {
    /// Schnorr proof `ψ_i`
    pub schnorr_proof: schnorr_pok::Proof<E>,
}

// ---------------------------------------------------------------------------
// Key share output
// ---------------------------------------------------------------------------

/// Output of the DKG: a party's key share and shared public data
#[derive(Debug, Clone)]
pub struct KeyShare<E: Curve> {
    /// This party's secret key share `x_i`
    pub secret_share: Scalar<E>,
    /// Combined public key `X = Σ X_j = x · G`
    pub public_key: Point<E>,
    /// All parties' public key shares `{X_j}` (indexed by party index)
    pub public_shares: Vec<NonZero<Point<E>>>,
    /// This party's index
    pub party_index: u16,
}

// ---------------------------------------------------------------------------
// Structured hashing (domain-separated, unambiguous encoding)
// ---------------------------------------------------------------------------

#[derive(udigest::Digestable)]
#[udigest(tag = tag!("hash_commitment"))]
#[udigest(bound = "")]
struct HashCommitInput<'a, E: Curve> {
    party_index: u16,
    decommitment: &'a MsgRound2<E>,
}

#[derive(udigest::Digestable)]
#[udigest(tag = tag!("schnorr_challenge"))]
#[udigest(bound = "")]
struct SchnorrChallengeInput<'a, E: Curve> {
    #[udigest(as_bytes)]
    rid: &'a [u8; 32],
    prover: u16,
    public_share: &'a NonZero<Point<E>>,
    schnorr_commit: &'a schnorr_pok::Commit<E>,
}

// ---------------------------------------------------------------------------
// Protocol execution
// ---------------------------------------------------------------------------

/// Runs the DKLs23 n-of-n distributed key generation protocol.
///
/// - `mpc`: round-based MPC engine for inter-party communication
/// - `i`: this party's index (0-based)
/// - `n`: total number of parties
/// - `rng`: cryptographically secure random number generator
///
/// Returns a [`KeyShare`] on success.
pub async fn keygen<E, D, R, M>(
    mut mpc: M,
    i: u16,
    n: u16,
    mut rng: R,
) -> Result<KeyShare<E>, ErrorM<M>>
where
    E: Curve,
    D: Digest + Clone + 'static,
    R: RngCore + CryptoRng,
    M: Mpc<Msg = Msg<E, D>>,
{
    // -----------------------------------------------------------------------
    // Setup
    // -----------------------------------------------------------------------
    let round1 = mpc.add_round(round_based::round::reliable_broadcast::<MsgRound1<D>>(i, n));
    let round2 = mpc.add_round(round_based::round::broadcast::<MsgRound2<E>>(i, n));
    let round3 = mpc.add_round(round_based::round::broadcast::<MsgRound3<E>>(i, n));
    let mut mpc = mpc.finish_setup();

    // -----------------------------------------------------------------------
    // Round 1 — Commit
    // -----------------------------------------------------------------------

    // Sample secret key share x_i
    let x_i = NonZero::<SecretScalar<E>>::random(&mut rng);
    let big_x_i = Point::generator() * &x_i;

    // Sample random identifier contribution
    let mut rid_i = [0u8; 32];
    (&mut rng).fill_bytes(&mut rid_i);

    // Generate Schnorr ephemeral commitment (A_i = α_i · G)
    let (schnorr_secret, schnorr_commit) =
        schnorr_pok::prover_commits_ephemeral_secret::<E, _>(&mut rng);

    // Decommitment nonce
    let mut decommit_nonce = [0u8; 32];
    (&mut rng).fill_bytes(&mut decommit_nonce);

    // Prepare decommitment (will be sent in Round 2)
    let my_decommitment = MsgRound2 {
        rid: rid_i,
        public_share: big_x_i,
        schnorr_commit: schnorr_commit.clone(),
        decommit_nonce,
    };

    // Compute commitment: V_i = H(i ‖ decommitment)
    let commitment = udigest::hash::<D>(&HashCommitInput {
        party_index: i,
        decommitment: &my_decommitment,
    });

    let my_commitment = MsgRound1 {
        commitment: commitment.clone(),
    };

    // Broadcast commitment (reliable broadcast to prevent equivocation)
    mpc.reliably_broadcast(Msg::Round1(my_commitment.clone()))
        .await
        .map_err(Error::Round1Send)?;

    // -----------------------------------------------------------------------
    // Round 2 — Decommit
    // -----------------------------------------------------------------------

    // Receive all commitments
    let commitments = mpc.complete(round1).await.map_err(Error::Round1Recv)?;

    // Broadcast our decommitment
    mpc.send_to_all(Msg::Round2(my_decommitment.clone()))
        .await
        .map_err(Error::Round2Send)?;

    // Receive all decommitments
    let decommitments = mpc.complete(round2).await.map_err(Error::Round2Recv)?;

    // Verify every commitment matches its decommitment
    let mut blame_decommit = Vec::new();
    for ((j, com_id, commit_j), (_, decom_id, decommit_j)) in commitments
        .into_iter_indexed()
        .zip(decommitments.iter_indexed())
    {
        let expected = udigest::hash::<D>(&HashCommitInput {
            party_index: j,
            decommitment: decommit_j,
        });
        if commit_j.commitment != expected {
            blame_decommit.push(Blame {
                guilty_party: j,
                msg1: com_id,
                msg2: decom_id,
            });
        }
    }
    if !blame_decommit.is_empty() {
        return Err(Error::CommitmentMismatch {
            guilty: blame_decommit,
        });
    }

    // -----------------------------------------------------------------------
    // Round 3 — Prove knowledge of secret share
    // -----------------------------------------------------------------------

    // Compute joint random identifier: rid = ⊕ rid_j
    let mut rid = rid_i;
    for (_, _, decommit_j) in decommitments.iter_indexed() {
        for (r, rj) in rid.iter_mut().zip(&decommit_j.rid) {
            *r ^= rj;
        }
    }

    // Derive Schnorr challenge from (rid, i, X_i, A_i) via Fiat-Shamir
    let challenge_scalar = Scalar::<E>::from_hash::<D>(&SchnorrChallengeInput {
        rid: &rid,
        prover: i,
        public_share: &big_x_i,
        schnorr_commit: &my_decommitment.schnorr_commit,
    });
    let challenge = schnorr_pok::Challenge {
        nonce: challenge_scalar,
    };

    // Compute Schnorr proof
    let proof = schnorr_pok::prove(&schnorr_secret, &challenge, &x_i);

    // Broadcast proof
    mpc.send_to_all(Msg::Round3(MsgRound3 {
        schnorr_proof: proof,
    }))
    .await
    .map_err(Error::Round3Send)?;

    // Receive all proofs
    let proofs = mpc.complete(round3).await.map_err(Error::Round3Recv)?;

    // Verify every Schnorr proof
    let mut blame_proof = Vec::new();
    for ((j, _proof_id, proof_j), (_, _, decommit_j)) in
        proofs.into_iter_indexed().zip(decommitments.iter_indexed())
    {
        // Re-derive the challenge for party j
        let challenge_j = schnorr_pok::Challenge {
            nonce: Scalar::<E>::from_hash::<D>(&SchnorrChallengeInput {
                rid: &rid,
                prover: j,
                public_share: &decommit_j.public_share,
                schnorr_commit: &decommit_j.schnorr_commit,
            }),
        };

        if proof_j
            .schnorr_proof
            .verify(
                &decommit_j.schnorr_commit,
                &challenge_j,
                &decommit_j.public_share.into_inner(),
            )
            .is_err()
        {
            blame_proof.push(j);
        }
    }
    if !blame_proof.is_empty() {
        return Err(Error::InvalidSchnorrProof {
            guilty_parties: blame_proof,
        });
    }

    // -----------------------------------------------------------------------
    // Output — compute combined public key and assemble key share
    // -----------------------------------------------------------------------

    let mut public_shares: Vec<NonZero<Point<E>>> = Vec::with_capacity(n as usize);
    for idx in 0..n {
        if idx == i {
            public_shares.push(big_x_i);
        } else {
            let share = decommitments
                .iter_indexed()
                .find(|(j, _, _)| *j == idx)
                .map(|(_, _, d)| d.public_share);
            if let Some(s) = share {
                public_shares.push(s);
            }
        }
    }

    // Combined public key X = Σ X_j
    let public_key: Point<E> = public_shares.iter().map(|p| p.into_inner()).sum();

    // Extract the scalar value from NonZero<SecretScalar<E>>
    let secret_ref: &SecretScalar<E> = x_i.as_ref();
    let scalar_ref: &Scalar<E> = secret_ref.as_ref();
    let secret_scalar: Scalar<E> = *scalar_ref;

    Ok(KeyShare {
        secret_share: secret_scalar,
        public_key,
        public_shares,
        party_index: i,
    })
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Protocol error
#[derive(Debug, thiserror::Error)]
pub enum Error<RecvErr, SendErr> {
    /// Failed to send message in round 1
    #[error("round 1: failed to send commitment")]
    Round1Send(#[source] SendErr),
    /// Failed to receive messages in round 1
    #[error("round 1: failed to receive commitments")]
    Round1Recv(#[source] RecvErr),
    /// Failed to send message in round 2
    #[error("round 2: failed to send decommitment")]
    Round2Send(#[source] SendErr),
    /// Failed to receive messages in round 2
    #[error("round 2: failed to receive decommitments")]
    Round2Recv(#[source] RecvErr),
    /// Failed to send message in round 3
    #[error("round 3: failed to send proof")]
    Round3Send(#[source] SendErr),
    /// Failed to receive messages in round 3
    #[error("round 3: failed to receive proofs")]
    Round3Recv(#[source] RecvErr),
    /// One or more parties' decommitments don't match their commitments
    #[error("commitment verification failed for {guilty:?}")]
    CommitmentMismatch {
        /// Parties whose commitments didn't match
        guilty: Vec<Blame>,
    },
    /// One or more parties provided invalid Schnorr proofs
    #[error("invalid Schnorr proof from parties {guilty_parties:?}")]
    InvalidSchnorrProof {
        /// Party indices with invalid proofs
        guilty_parties: Vec<u16>,
    },
}

/// Error type parameterized by the MPC engine
pub type ErrorM<M> = Error<
    round_based::mpc::CompleteRoundErr<M, round_based::round::RoundInputError>,
    <M as Mpc>::SendErr,
>;

/// Identifies a misbehaving party with evidence
#[derive(Debug)]
pub struct Blame {
    /// Index of the misbehaving party
    pub guilty_party: u16,
    /// Message ID of the commitment
    pub msg1: MsgId,
    /// Message ID of the decommitment
    pub msg2: MsgId,
}

// ---------------------------------------------------------------------------
// Serde helper for fixed-size byte arrays
// ---------------------------------------------------------------------------

mod hex_or_bytes {
    use alloc::vec::Vec;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error> {
        bytes.as_slice().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 32], D::Error> {
        let v = <Vec<u8>>::deserialize(deserializer)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom("expected 32 bytes"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use generic_ec::{Point, Scalar};

    fn run_keygen<E: Curve>(n: u16) -> Vec<KeyShare<E>> {
        let mut rng = rand_dev::DevRng::new();
        round_based::sim::run_with_setup(
            core::iter::repeat_with(|| rng.fork()).take(n.into()),
            |i, party, rng| keygen::<E, sha2::Sha256, _, _>(party, i, n, rng),
        )
        .unwrap()
        .expect_ok()
        .into_iter()
        .collect()
    }

    fn verify_key_shares<E: Curve>(key_shares: &[KeyShare<E>]) {
        let pk = key_shares[0].public_key;

        // All parties agree on the same public key
        for share in key_shares {
            assert_eq!(
                share.public_key, pk,
                "public key mismatch at party {}",
                share.party_index
            );
        }

        // Secret shares reconstruct the full secret: Σ x_i · G = X
        let reconstructed: Scalar<E> = key_shares.iter().map(|s| s.secret_share).sum();
        assert_eq!(
            Point::generator() * reconstructed,
            pk,
            "secret shares do not reconstruct the public key"
        );

        // Each party's public share matches its secret share
        for share in key_shares {
            let expected = Point::generator() * share.secret_share;
            assert_eq!(
                share.public_shares[share.party_index as usize].into_inner(),
                expected,
                "public share mismatch for party {}",
                share.party_index
            );
        }
    }

    #[test]
    fn keygen_2_of_2_secp256k1() {
        use generic_ec::curves::Secp256k1;
        let shares = run_keygen::<Secp256k1>(2);
        verify_key_shares(&shares);
    }

    #[test]
    fn keygen_3_of_3_secp256k1() {
        use generic_ec::curves::Secp256k1;
        let shares = run_keygen::<Secp256k1>(3);
        verify_key_shares(&shares);
        std::println!("DKLs23 3-of-3 public key: {:?}", shares[0].public_key);
    }

    #[test]
    fn keygen_5_of_5_secp256k1() {
        use generic_ec::curves::Secp256k1;
        let shares = run_keygen::<Secp256k1>(5);
        verify_key_shares(&shares);
    }

    #[test]
    fn keygen_3_of_3_secp256r1() {
        use generic_ec::curves::Secp256r1;
        let shares = run_keygen::<Secp256r1>(3);
        verify_key_shares(&shares);
    }

    #[tokio::test]
    async fn keygen_async_3_of_3() {
        use generic_ec::curves::Secp256k1;
        let mut rng = rand_dev::DevRng::new();
        let n: u16 = 3;

        let key_shares: Vec<KeyShare<Secp256k1>> = round_based::sim::async_env::run_with_setup(
            core::iter::repeat_with(|| rng.fork()).take(n.into()),
            |i, party, rng| keygen::<Secp256k1, sha2::Sha256, _, _>(party, i, n, rng),
        )
        .await
        .expect_ok()
        .into_iter()
        .collect();

        verify_key_shares(&key_shares);
    }
}
